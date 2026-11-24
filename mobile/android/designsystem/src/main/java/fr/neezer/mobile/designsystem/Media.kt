package fr.neezer.mobile.designsystem

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import uniffi.nz_ffi.TrackRow

// -- the Sillon groove --------------------------------------------------------
//
// `webapp/src/lib/sillon.js`: eight dark pairs, chosen by FNV-1a over the seed,
// painted as a repeating radial grain. Deterministic on purpose — a placeholder
// that changed colour on reload would read as a loading state — and the accent
// tints of the SPA's `grooveTint` for an artist without a photo.

/** FNV-1a, 32-bit — the SPA's `hashId`, four lines and no table. */
fun nzHashId(seed: String?): Long {
    var h = 0x811C9DC5L
    val s = seed ?: ""
    for (ch in s) {
        h = h xor ch.code.toLong()
        h = (h * 0x01000193L) and 0xFFFFFFFFL
    }
    return h
}

/** The colour pair of a missing cover or an artist's tile. Same seed, same pair, forever. */
fun nzGrooveFor(seed: String?): Pair<Color, Color> =
    NzGrooves.pairs[(nzHashId(seed) % NzGrooves.pairs.size).toInt()]

/** Up to two initials: "Dr. Peacock" → DP, "Gorillaz" → GO. */
fun nzInitialsOf(name: String?): String {
    val words = (name ?: "")
        .filter { it.isLetterOrDigit() || it == ' ' }
        .split(' ')
        .filter { it.isNotEmpty() }
    if (words.isEmpty()) return ""
    if (words.size == 1) return words[0].take(2).uppercase()
    return (words[0].take(1) + words[1].take(1)).uppercase()
}

/**
 * Where a cover's pixels come from.
 *
 * Rust fetches and caches the file and Android decodes it (docs/agents/mobile.md,
 * "Covers"): the design system cannot know about Coil, a Core, or a network
 * lane, so it declares this seam and `:bridge` (MB2) provides the Coil loader.
 * With no loader, a cover is its Sillon groove — never an empty square, and
 * never a request (rule 1: the design system asks for nothing).
 */
interface NzCoverLoader {
    /** Draw `url` at `sizePx`; the caller has already painted the groove under it. */
    @Composable
    fun Image(url: String, sizePx: Int)
}

val LocalNzCoverLoader = staticCompositionLocalOf<NzCoverLoader?> { null }

/**
 * A sleeve. The deterministic groove paints from the FIRST frame and the image
 * sits on it (`webapp.md`: "a queued image sits ON it"), so a slow cover never
 * leaves a blank square.
 */
@Composable
fun CoverImage(
    url: String?,
    sizeDp: Dp,
    modifier: Modifier = Modifier,
    shape: Shape = RoundedCornerShape(NzRadius.cover),
    seed: String? = null,
) {
    val loader = LocalNzCoverLoader.current
    // The requested pixels are the DRAWN pixels: a 3× guess decoded a sleeve
    // larger (or smaller) than the screen on every density. The loader's Coil
    // key includes this size, so the same sleeve in a 44 dp row and on the
    // player's 420 dp stage are two entries, each at its own size.
    val sizePx = with(LocalDensity.current) { sizeDp.roundToPx() }
    val (a, b) = nzGrooveFor(seed ?: url)
    Box(modifier = modifier.size(sizeDp).clip(shape)) {
        Canvas(Modifier.size(sizeDp)) {
            drawRect(color = a)
            // repeating-radial-gradient(circle at 68% 32%, a 0 5px, b 5px 6px)
            val cx = size.width * 0.68f
            val cy = size.height * 0.32f
            val step = 6.dp.toPx()
            var radius = 0f
            val maxRadius = size.maxDimension
            while (radius < maxRadius) {
                drawCircle(color = b, radius = radius + step * 0.9f, center = Offset(cx, cy), style = Stroke(step * 0.15f))
                radius += step
            }
        }
        if (url != null && loader != null) {
            loader.Image(url, sizePx)
        }
    }
}

/**
 * An artist's tile: the initials on the groove's tint, UNDER the photo, so a
 * portrait that is still loading is an initialled disc and not a hole.
 */
@Composable
fun Avatar(
    name: String,
    sizeDp: Dp,
    modifier: Modifier = Modifier,
    url: String? = null,
) {
    val (a, b) = nzGrooveFor(name)
    val loader = LocalNzCoverLoader.current
    val sizePx = with(LocalDensity.current) { sizeDp.roundToPx() }
    Box(
        modifier = modifier
            .size(sizeDp)
            .clip(CircleShape),
        contentAlignment = Alignment.Center,
    ) {
        Canvas(Modifier.size(sizeDp)) {
            drawRect(
                brush = Brush.radialGradient(
                    colors = listOf(b, a),
                    center = Offset(size.width * 0.28f, size.height * 0.18f),
                    radius = size.maxDimension * 1.2f,
                ),
            )
        }
        Text(
            text = nzInitialsOf(name),
            style = if (sizeDp >= 56.dp) Sillon.type.titleSmall else Sillon.type.label,
            color = NzGrooves.ink,
        )
        if (url != null && loader != null) {
            loader.Image(url, sizePx)
        }
    }
}

/**
 * The quality a title is streamed at: a mono badge on a hairline (the SPA's
 * `FLAC` / `OPUS 320` pill). `--dim` is the ink: a badge is metadata, not a
 * state, so it carries no accent.
 */
@Composable
fun QualityBadge(label: String, modifier: Modifier = Modifier) {
    val colors = Sillon.colors
    Box(
        modifier = modifier
            .clip(RoundedCornerShape(NzRadius.chip))
            .border(1.dp, colors.ln, RoundedCornerShape(NzRadius.chip))
            .padding(horizontal = NzSpacing.s, vertical = NzSpacing.xxs),
    ) {
        Text(text = label.uppercase(), style = Sillon.type.mono, color = colors.dim)
    }
}

/**
 * One row of any track list (the SPA's shared list shape, on a phone): a 44 dp
 * sleeve, the title and its artists, the duration in mono figures, and one ⋯
 * control. The whole row is the target (56 dp tall).
 *
 * * the playing row is `--ac-text` — an accent FOREGROUND, which on Papier is
 *   the ink, never the lemon (1.16:1);
 * * **swipe right = play next** (`webapp.md`): the row follows the finger, a
 *   "Lire ensuite" hint appears behind it, and a release past 72 dp asks
 *   [onSwipeQueue] and springs back — the queue itself is the caller's;
 * * a row never animates on mount (M4): recycled rows must not replay an
 *   entrance, and a 4 000-row queue would replay it forever.
 */
@Composable
fun TrackRowView(
    row: TrackRow,
    playing: Boolean,
    onClick: () -> Unit,
    onMore: () -> Unit,
    onSwipeQueue: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val colors = Sillon.colors
    var drag by remember { mutableFloatStateOf(0f) }
    val threshold = 72.dp
    Box(modifier = modifier.fillMaxWidth()) {
        // Behind the row: what a swipe will do, said in words, not in colour.
        Text(
            text = "Lire ensuite",
            style = Sillon.type.label,
            color = colors.acText,
            modifier = Modifier
                .align(Alignment.CenterStart)
                .padding(start = NzSpacing.l),
        )
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .height(56.dp)
                .graphicsLayer { translationX = drag }
                .background(colors.bg)
                .pointerInput(Unit) {
                    detectHorizontalDragGestures(
                        onDragEnd = {
                            val passed = drag > threshold.toPx()
                            drag = 0f
                            if (passed) onSwipeQueue()
                        },
                        onDragCancel = { drag = 0f },
                    ) { change, amount ->
                        // Right only: a left swipe belongs to the page's back gesture.
                        drag = (drag + amount).coerceIn(0f, size.width.toFloat() * 0.75f)
                        change.consume()
                    }
                }
                .clickable(role = Role.Button, onClick = onClick)
                .padding(horizontal = NzSpacing.s, vertical = NzSpacing.xs),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
        ) {
            Box(contentAlignment = Alignment.Center) {
                CoverImage(url = row.cover, sizeDp = 44.dp, seed = row.id)
                if (playing) {
                    Box(
                        modifier = Modifier
                            .size(24.dp)
                            .clip(CircleShape)
                            .background(colors.bg.copy(alpha = 0.72f)),
                        contentAlignment = Alignment.Center,
                    ) {
                        Icon(glyph = NzGlyph.Play, size = 12.dp, tint = colors.acText, contentDescription = null)
                    }
                }
            }
            Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(
                    text = row.title,
                    style = Sillon.type.body,
                    color = if (playing) colors.acText else colors.tx,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    text = row.artists,
                    style = Sillon.type.bodySmall,
                    color = colors.dim,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            Text(
                text = nzDuration(row.durationS.toInt()),
                style = Sillon.type.mono,
                color = colors.dim,
            )
            NzIconButton(
                glyph = NzGlyph.MoreVertical,
                contentDescription = "Plus d'actions pour ${row.title}",
                onClick = onMore,
            )
        }
    }
}

/** `m:ss` — a duration as a row shows it, in the mono face. */
fun nzDuration(seconds: Int): String {
    val s = seconds.coerceAtLeast(0)
    return "${s / 60}:${(s % 60).toString().padStart(2, '0')}"
}
