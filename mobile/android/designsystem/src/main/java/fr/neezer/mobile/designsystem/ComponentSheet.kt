package fr.neezer.mobile.designsystem

import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.layout.wrapContentHeight
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import uniffi.nz_ffi.TrackRow

/**
 * Every component of the design system, in one column: the sheet Android
 * Studio's preview shows and the subject of the MB1 screenshots
 * (`docs/agents/screenshots/mobile/MB1/`).
 *
 * It exists because rule 5 is a VISUAL rule: a screenshot of this sheet in one
 * theme at one width is how a person sees that nothing fell back to a default
 * widget — and it is how the first version's invisible « Lire » (ink on ink on
 * Papier) was caught. A screen lot that doubts a token opens it.
 */
/** A sample row for the sheet and for a preview — what the FFI hands a track list. */
fun nzDemoRow(index: Int): TrackRow = TrackRow(
    key = "row:$index",
    id = "$index",
    title = "Titre $index",
    artists = "Artiste ${index % 7}",
    album = "Album ${index % 13}",
    albumId = null,
    durationS = (120 + index % 60).toUInt(),
    cover = null,
    explicit = false,
    liked = index % 5 == 0,
    disliked = false,
    unavailable = false,
    downloaded = false,
    source = "deezer",
    position = index.toUInt(),
)

@Composable
fun NzComponentSheet(width: Int) {
    val colors = Sillon.colors
    Column(
        modifier = Modifier.fillMaxWidth().background(colors.bg).padding(bottom = NzSpacing.xxl),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.l),
    ) {
        NzTopBar(title = "Bibliothèque", onBack = {}) {
            NzIconButton(NzGlyph.Search, "Rechercher", onClick = {})
            NzIconButton(NzGlyph.MoreVertical, "Plus", onClick = {})
        }

        Section("Actions") {
            // Three buttons do not fit one 360 dp line: a screen wraps them, it
            // never lets a pill grow a second line (measured on this gallery).
            Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
                NzButton(text = "Lire", onClick = {}, glyph = NzGlyph.Play)
                NzButton(text = "Mélanger", onClick = {}, kind = NzButtonKind.Secondary, glyph = NzGlyph.Shuffle)
            }
            Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
                NzButton(text = "Télécharger", onClick = {}, kind = NzButtonKind.Ghost, glyph = NzGlyph.Download)
                NzButton(text = "Indisponible", onClick = {}, enabled = false)
            }
            Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.xs)) {
                NzIconButton(NzGlyph.Heart, "Aimer", onClick = {})
                NzIconButton(NzGlyph.HeartFilled, "Aimé", onClick = {}, selected = true)
                NzIconButton(NzGlyph.More, "Actions", onClick = {})
                NzIconButton(NzGlyph.Plus, "Ajouter", onClick = {})
                NzIconButton(NzGlyph.Download, "Télécharger", onClick = {})
                NzIconButton(NzGlyph.Play, "Lire ce titre", onClick = {})
            }
        }

        Section("Choix") {
            Row(
                horizontalArrangement = Arrangement.spacedBy(NzSpacing.s),
                modifier = Modifier.horizontalScroll(rememberScrollState()),
            ) {
                NzChip(text = "Tout", onClick = {}, selected = true)
                NzChip(text = "Titres", onClick = {})
                NzChip(text = "Albums", onClick = {})
                NzChip(text = "Hors-ligne", onClick = {}, glyph = NzGlyph.Downloaded)
            }
            NzSegmented(
                options = listOf("Musique", "Podcasts"),
                selectedIndex = 0,
                onSelect = {},
                modifier = Modifier.fillMaxWidth(),
            )
            Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.l)) {
                NzSwitch(checked = true, onCheckedChange = {}, label = "Animations")
                NzSwitch(checked = false, onCheckedChange = {}, label = "Mode éco")
            }
        }

        Section("Titres") {
            TrackRowView(
                row = nzDemoRow(3).copy(title = "Nuit blanche", artists = "Kavinsky, Phoenix", durationS = 244u),
                playing = true,
                onClick = {},
                onMore = {},
                onSwipeQueue = {},
            )
            TrackRowView(
                row = nzDemoRow(4).copy(title = "La femme d'argent", artists = "Air", durationS = 425u),
                playing = false,
                onClick = {},
                onMore = {},
                onSwipeQueue = {},
            )
            TrackRowView(
                row = nzDemoRow(5).copy(title = "Un titre dont le nom est beaucoup trop long pour une seule ligne de téléphone", artists = "Artiste au nom tout aussi long", durationS = 61u),
                playing = false,
                onClick = {},
                onMore = {},
                onSwipeQueue = {},
            )
        }

        Section("Pochettes et figures") {
            // A rail that FITS: a Row gives its last child the remaining width, so
            // five covers on one 360 dp line clipped the last avatar (seen in the
            // screenshots). Two lines, nothing cut.
            Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.m), verticalAlignment = Alignment.CenterVertically) {
                CoverImage(url = null, sizeDp = 96.dp, seed = "album:1")
                CoverImage(url = null, sizeDp = 64.dp, seed = "album:2")
                CoverImage(url = null, sizeDp = 44.dp, seed = "album:3", shape = RoundedCornerShape(NzRadius.field))
            }
            Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.m), verticalAlignment = Alignment.CenterVertically) {
                Avatar(name = "Dr. Peacock", sizeDp = 56.dp)
                Avatar(name = "Gorillaz", sizeDp = 40.dp)
                Avatar(name = "Air", sizeDp = 32.dp)
            }
            Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.s), verticalAlignment = Alignment.CenterVertically) {
                QualityBadge("FLAC")
                QualityBadge("Opus 320")
                QualityBadge("128")
                Text("12:34", style = Sillon.type.mono, color = colors.dim)
                Text("LABEL MONO", style = Sillon.type.labelMono, color = colors.dim)
            }
        }

        Section("Réglages : égaliseur") {
            Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
                listOf(0.15f, 0.5f, 0.9f).forEach { value ->
                    NzFader(value = value, onValueChange = {}, label = "Bande ${(value * 100).toInt()}", height = 120.dp)
                }
            }
        }

        Section("États") {
            Skeleton(kind = NzSkeletonKind.List, count = 3)
            LoadFailed(cause = "le serveur n'a pas répondu dans les 15 s", onRetry = {})
            EmptyState(
                title = "Aucune playlist",
                body = "Crée une liste pour commencer ta collection.",
                actionLabel = "Nouvelle playlist",
                onAction = {},
            )
            NetworkIndicator(NzNetwork.Offline)
            NetworkIndicator(NzNetwork.Reconnected)
            Box(Modifier.fillMaxWidth().height(96.dp)) { NzToastHost(bottomPadding = 0.dp) }
        }

        Section("Glyphes") {
            NzGlyph.entries.chunked(if (width >= 600) 18 else 12).forEach { row ->
                Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.m), modifier = Modifier.padding(vertical = 2.dp)) {
                    row.forEach { glyph -> Icon(glyph = glyph, size = 18.dp, tint = colors.dim) }
                }
            }
        }

        Section("Typographie") {
            Text("Bricolage Grotesque 44", style = Sillon.type.display.copy(fontSize = 30.sp), color = colors.tx)
            Text("Section 24", style = Sillon.type.title, color = colors.tx)
            Text("Titre de carte 19", style = Sillon.type.titleSmall, color = colors.tx)
            Text("Corps 16 — Ton archive musicale, gravée à ta façon.", style = Sillon.type.body, color = colors.tx)
            Text("Secondaire 14", style = Sillon.type.bodySmall, color = colors.dim)
            Text("ÉTIQUETTE", style = Sillon.type.label, color = colors.tx)
            Text("MONO 11 · 3:24", style = Sillon.type.mono, color = colors.dim)
        }
    }
}

/** A gallery section: a mono label and its content, like a spec sheet. */
@Composable
private fun Section(title: String, content: @Composable () -> Unit) {
    val colors = Sillon.colors
    Column(
        modifier = Modifier.fillMaxWidth().padding(horizontal = NzSpacing.pageX).wrapContentHeight(),
        verticalArrangement = Arrangement.spacedBy(NzSpacing.s),
    ) {
        Text(text = title.uppercase(), style = Sillon.type.labelMono, color = colors.dim)
        content()
    }
}

/** A theme's name as the sheet prints it. */
fun NzThemeKind.label(): String = when (this) {
    NzThemeKind.Encre -> "Encre"
    NzThemeKind.Papier -> "Papier"
    NzThemeKind.Pochette -> "Pochette"
}

// -- Android Studio previews --------------------------------------------------
//
// One per theme at the two widths the screenshots use, so the sheet opens in the
// IDE exactly as it is committed. The screenshot test renders the same sheet on
// the JVM (Robolectric, real pixels), which is what keeps them from drifting.

@androidx.compose.ui.tooling.preview.Preview(name = "Encre 360", showBackground = true, widthDp = 360, heightDp = 2100)
@Composable
private fun PreviewEncre() {
    NzTheme(kind = NzThemeKind.Encre) { NzComponentSheet(width = 360) }
}

@androidx.compose.ui.tooling.preview.Preview(name = "Papier 360", showBackground = true, widthDp = 360, heightDp = 2100)
@Composable
private fun PreviewPapier() {
    NzTheme(kind = NzThemeKind.Papier) { NzComponentSheet(width = 360) }
}

@androidx.compose.ui.tooling.preview.Preview(name = "Pochette 360", showBackground = true, widthDp = 360, heightDp = 2100)
@Composable
private fun PreviewPochette() {
    NzTheme(kind = NzThemeKind.Pochette) { NzComponentSheet(width = 360) }
}

@androidx.compose.ui.tooling.preview.Preview(name = "Encre 600", showBackground = true, widthDp = 600, heightDp = 2000)
@Composable
private fun PreviewEncreWide() {
    NzTheme(kind = NzThemeKind.Encre) { NzComponentSheet(width = 600) }
}
