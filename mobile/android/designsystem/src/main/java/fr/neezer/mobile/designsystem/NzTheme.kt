package fr.neezer.mobile.designsystem

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp

/**
 * The three Sillon themes (`encre` dark with the lemon accent, `papier` warm
 * light with ink, `pochette` dark with the cover's accent).
 *
 * **MB0 placeholder**: only the colours the shell paints with, copied from
 * `webapp/src/app.css` so the skeleton already looks like Neezer rather than
 * like a default Compose app. MB1 owns this file and replaces it with the exact
 * tokens, the three fonts, the motion table and the eco/reduced guard.
 */
enum class NzThemeKind { Encre, Papier, Pochette }

/**
 * The tokens as far as MB0 needs them. Every screen reads these, never a
 * literal colour (rule 5): a theme switch must repaint without touching a
 * component.
 */
object Sillon {
    // encre (the default): the app's ground, its two surfaces, the text steps.
    val bg = Color(0xFF0C0C0B)
    val s1 = Color(0xFF151513)
    val s2 = Color(0xFF1E1E1B)
    val line = Color(0xFF2A2A26)
    val tx = Color(0xFFF1EFE8)
    val dim = Color(0xFFA09D92)
    /** A SURFACE: text drawn on it is [onAccent], never white. */
    val accent = Color(0xFFD2FF3C)
    val onAccent = Color(0xFF0C0C0B)

    val space = 16.dp
    val gap = 8.dp
}

private val EncreScheme = darkColorScheme(
    primary = Sillon.accent,
    onPrimary = Sillon.onAccent,
    background = Sillon.bg,
    onBackground = Sillon.tx,
    surface = Sillon.s1,
    onSurface = Sillon.tx,
    surfaceVariant = Sillon.s2,
    onSurfaceVariant = Sillon.dim,
    outline = Sillon.line,
)

private val PapierScheme = lightColorScheme(
    primary = Sillon.bg,
    onPrimary = Sillon.tx,
    background = Color(0xFFF6F3EA),
    onBackground = Color(0xFF14140F),
    surface = Color(0xFFFFFDF7),
    onSurface = Color(0xFF14140F),
    surfaceVariant = Color(0xFFEDE8DA),
    onSurfaceVariant = Color(0xFF5C574B),
    outline = Color(0xFFD6D0C0),
)

/** `pochette` follows the playing cover; MB0 paints it as `encre`. */
private val PochetteScheme = EncreScheme

private val LocalNzThemeKind = staticCompositionLocalOf { NzThemeKind.Encre }

/** Which theme a component is drawn in (the eco/reduced guard arrives with MB1). */
val currentNzTheme: NzThemeKind
    @Composable get() = LocalNzThemeKind.current

/**
 * The app's one theme. [accent] is a custom accent from Réglages › Apparence
 * (`null` keeps the theme's own); [coverAccent] is Pochette's.
 */
@Composable
fun NzTheme(
    kind: NzThemeKind = NzThemeKind.Encre,
    accent: Color? = null,
    coverAccent: Color? = null,
    content: @Composable () -> Unit,
) {
    val base = when (kind) {
        NzThemeKind.Encre -> EncreScheme
        NzThemeKind.Papier -> PapierScheme
        NzThemeKind.Pochette -> PochetteScheme
    }
    val chosen = accent ?: coverAccent
    val scheme = if (chosen == null) base else base.copy(primary = chosen)
    CompositionLocalProvider(LocalNzThemeKind provides kind) {
        MaterialTheme(colorScheme = scheme, content = content)
    }
}

/**
 * What a route whose lot has not landed yet shows: its name, and the fact that
 * it is being built. Every entry composable draws this in MB0, so "every route
 * opens" is something a person can see and a screen reader can read.
 */
@Composable
fun NzPlaceholder(title: String, body: String, modifier: Modifier = Modifier) {
    Column(
        modifier = modifier
            .fillMaxSize()
            .background(MaterialTheme.colorScheme.background)
            .padding(Sillon.space),
        verticalArrangement = Arrangement.spacedBy(Sillon.gap, Alignment.CenterVertically),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(text = title, style = MaterialTheme.typography.headlineSmall, textAlign = TextAlign.Center)
        Text(
            text = body,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )
    }
}
