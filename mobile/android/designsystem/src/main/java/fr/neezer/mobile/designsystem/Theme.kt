package fr.neezer.mobile.designsystem

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.Text
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.remember
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import android.provider.Settings
import androidx.compose.ui.text.style.TextAlign

/** The colours in force. Provided once by [NzTheme]; read it as `Sillon.colors`. */
val LocalNzColors = staticCompositionLocalOf { Sillon.theme(NzThemeKind.Encre) }

/** The type scale in force. */
val LocalNzTypography = staticCompositionLocalOf { NzTypographyTable }

/**
 * The motion level in force. Defaults to [NzMotionLevel.Full] so a preview or a
 * test that renders one component still works; [NzTheme] replaces it with the
 * device's setting.
 */
val LocalNzMotion = staticCompositionLocalOf { NzMotion(NzMotionLevel.Full) }

/** Which theme is painted — a screen that draws a theme-specific thing reads this. */
val LocalNzThemeKind = staticCompositionLocalOf { NzThemeKind.Encre }

/** Which theme is painted (Réglages › Apparence shows the choice). */
val currentNzTheme: NzThemeKind
    @Composable get() = LocalNzThemeKind.current

private fun nzColorScheme(c: NzColors) = when (c.isDark) {
    true -> darkColorScheme(
        primary = c.ac,
        onPrimary = c.onac,
        primaryContainer = c.ac,
        onPrimaryContainer = c.onac,
        secondary = c.btn,
        onSecondary = c.onbtn,
        secondaryContainer = c.s2,
        onSecondaryContainer = c.tx,
        tertiary = c.acMark,
        onTertiary = c.onac,
        background = c.bg,
        onBackground = c.tx,
        surface = c.s1,
        onSurface = c.tx,
        surfaceVariant = c.s2,
        onSurfaceVariant = c.dim,
        surfaceContainer = c.s1,
        surfaceContainerHigh = c.s2,
        surfaceContainerHighest = c.s2,
        surfaceContainerLow = c.s1,
        surfaceContainerLowest = c.bg,
        outline = c.ln,
        outlineVariant = c.ln,
        scrim = c.scrim,
        // Sillon has NO error colour: a failure is a neutral block with its
        // cause and « Réessayer » (rule 3), never a red surface (app.css).
        error = c.tx,
        onError = c.s2,
        errorContainer = c.s2,
        onErrorContainer = c.tx,
        inverseSurface = c.tx,
        inverseOnSurface = c.bg,
    )
    false -> lightColorScheme(
        primary = c.ac,
        onPrimary = c.onac,
        primaryContainer = c.ac,
        onPrimaryContainer = c.onac,
        secondary = c.btn,
        onSecondary = c.onbtn,
        secondaryContainer = c.s2,
        onSecondaryContainer = c.tx,
        tertiary = c.acMark,
        onTertiary = c.onac,
        background = c.bg,
        onBackground = c.tx,
        surface = c.s1,
        onSurface = c.tx,
        surfaceVariant = c.s2,
        onSurfaceVariant = c.dim,
        surfaceContainer = c.s1,
        surfaceContainerHigh = c.s2,
        surfaceContainerHighest = c.s2,
        surfaceContainerLow = c.s1,
        surfaceContainerLowest = c.bg,
        outline = c.ln,
        outlineVariant = c.ln,
        scrim = c.scrim,
        error = c.tx,
        onError = c.s2,
        errorContainer = c.s2,
        onErrorContainer = c.tx,
        inverseSurface = c.tx,
        inverseOnSurface = c.bg,
    )
}

private fun nzTypography(t: NzTypography) = Typography(
    displayLarge = t.display,
    displayMedium = t.displaySmall,
    displaySmall = t.title,
    headlineLarge = t.title,
    headlineMedium = t.titleSmall,
    headlineSmall = t.titleSmall,
    titleLarge = t.title,
    titleMedium = t.titleSmall,
    titleSmall = t.label,
    bodyLarge = t.body,
    bodyMedium = t.bodySmall,
    bodySmall = t.labelSmall,
    labelLarge = t.label,
    labelMedium = t.labelSmall,
    labelSmall = t.mono,
)

private val NzShapes = Shapes(
    extraSmall = RoundedCornerShape(NzRadius.field),
    small = RoundedCornerShape(NzRadius.field),
    medium = RoundedCornerShape(NzRadius.cover),
    large = RoundedCornerShape(NzRadius.card),
    extraLarge = RoundedCornerShape(NzRadius.card),
)

/**
 * The app's ONE theme.
 *
 * @param kind `encre` (the default), `papier` or `pochette`.
 * @param accent a custom accent from Réglages › Apparence (`null` keeps the
 *   theme's own). It never applies to Pochette, whose accent IS the sleeve.
 * @param coverAccent Pochette's `--cover-ac`: the playing sleeve's dominant
 *   colour, already lifted by the caller or lifted here ([nzReadableInk]).
 * @param motion `null` reads the device (Android's animator duration scale, the
 *   platform's own reduced-motion setting); [NzMotionLevel.Off] is eco mode.
 *
 * Everything Material draws underneath is re-skinned here (rule 5): a screen
 * never sees a default Material colour or typeface.
 */
@Composable
fun NzTheme(
    kind: NzThemeKind = NzThemeKind.Encre,
    accent: Color? = null,
    coverAccent: Color? = null,
    motion: NzMotionLevel? = null,
    content: @Composable () -> Unit,
) {
    val colors = remember(kind, accent, coverAccent) { Sillon.theme(kind, accent, coverAccent) }
    // Android's own reduced-motion signal is the animator duration scale: 0 when
    // « Supprimer les animations » is on. Read from the resolver, so a preview
    // and a headless test get the system default instead of a crash.
    val context = LocalContext.current
    val systemScale = remember(context) {
        runCatching {
            Settings.Global.getFloat(context.contentResolver, Settings.Global.ANIMATOR_DURATION_SCALE, 1f)
        }.getOrDefault(1f)
    }
    val level = motion ?: NzMotionLevel.fromSystem(systemScale)
    val tokens = remember(level) { NzMotion(level) }
    val scheme = remember(colors) { nzColorScheme(colors) }
    val type = remember { nzTypography(NzTypographyTable) }
    CompositionLocalProvider(
        LocalNzColors provides colors,
        LocalNzTypography provides NzTypographyTable,
        LocalNzMotion provides tokens,
        LocalNzThemeKind provides kind,
    ) {
        MaterialTheme(colorScheme = scheme, typography = type, shapes = NzShapes, content = content)
    }
}

/**
 * What a route whose lot has not landed yet shows: its name, and the fact that
 * it is being built. Every entry composable draws this until its lot replaces
 * it, so "every route opens" is something a person can see and a screen reader
 * can read.
 */
@Composable
fun NzPlaceholder(title: String, body: String, modifier: Modifier = Modifier) {
    val colors = Sillon.colors
    Column(
        modifier = modifier
            .fillMaxSize()
            .background(colors.bg)
            .padding(Sillon.space),
        verticalArrangement = Arrangement.spacedBy(Sillon.gap, Alignment.CenterVertically),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(text = title, style = Sillon.type.title, color = colors.tx, textAlign = TextAlign.Center)
        Text(
            text = body,
            style = Sillon.type.bodySmall,
            color = colors.dim,
            textAlign = TextAlign.Center,
        )
    }
}
