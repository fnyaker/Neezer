package fr.neezer.mobile.feature.viz

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.text.TextMeasurer
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.drawText
import androidx.compose.ui.unit.sp
import kotlin.math.PI
import kotlin.math.abs
import kotlin.math.exp
import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToInt
import kotlin.math.sin
import kotlin.math.sqrt
import uniffi.nz_ffi.VizFrame

/**
 * The bars and the oscilloscope, drawn from the numbers Rust published — the
 * drawing half of `webapp/src/lib/viz/scenes/bars.js` and `scope.js`, ported.
 *
 * The LEVELLING is Rust's (`nz-viz`, the page's own `viz_bars`): the three-zone
 * auto-gain, the gamma, the envelopes and the peaks arrive in the frame. What
 * this file owns is what the browser's scene owns — where a capsule sits, how
 * wide it is, which colour it takes from the sleeve's walk, and how the trace's
 * three strokes build a beam. Every constant below is the Svelte file's, with
 * the line it comes from.
 *
 * # The two layouts
 *
 * `layout = "full"` is the visualiser screen (`reach = h * 0.31`, the row
 * grounded on the axis). `layout = "strip"` is the player's thin row (the whole
 * half-height, one flat colour — `bars.js`'s `flat` path, the cheapest row
 * there is). The full screen keeps the sleeve's own colours.
 *
 * # Papier paints ink
 *
 * The SPA's full screen is a dark room on every theme, so `bars.js` only takes
 * the ink path for the player's strip on Papier. A phone cannot have a cream
 * page with a lemon row on it: the full screen follows the app's theme (rule 5,
 * and the acceptance's two themes), so on a light theme the capsules are INK —
 * the same two fills `bars.js#ink` uses, at its own values. That is the one
 * deliberate departure, and it is the Svelte file's own ink branch.
 */
internal object VizDraw {
    /** `bars.js#BREATH_BEATS`: one cycle of breath every four beats. */
    private const val BREATH_BEATS = 4f
    private const val TAU = (PI * 2.0).toFloat()

    /** `scope.js#DEFLECT`: full scale stops short of the graticule. */
    private const val DEFLECT = 0.88f
    /** `scope.js#PERSIST`: the phosphor's time constant, in seconds. */
    private const val PERSIST = 0.045f
    private const val CORE_BRIGHT = 1.55f
    private const val HALO_BRIGHT = 0.13f
    /** `scope.js#FALLBACK.divisions` — the medium tier's count. */
    private const val DIVISIONS = 6

    // --- the bars ------------------------------------------------------------

    /**
     * `bars.js#draw`, for one frame.
     *
     * `beats` is the SPA's continuous beat position; MB21 has no tempo (the
     * spectrum depth), so the caller advances it with the SPA's own fallback
     * period and the rest breath keeps its 2 Hz.
     */
    fun bars(
        scope: DrawScope,
        frame: VizFrame,
        intensity: Float,
        full: Boolean,
        light: Boolean,
        eco: Boolean,
        reduced: Boolean,
        beats: Float,
    ) {
        val bars = frame.bars.size
        if (bars == 0) return
        val w = scope.size.width
        val h = scope.size.height
        if (w <= 0f || h <= 0f) return
        val ink = light
        // The strip on a dark theme: ONE flat colour, no hue walk, no afterglow —
        // `bars.js#flat`. (On Papier the ink branch paints deeper values instead;
        // a single lemon capsule on cream is the highlighter the design system
        // forbids.)
        val flat = !full && !light
        val bw = w / bars
        val body = max(2f, min(bw * 0.64f, h * 0.55f))
        val r = body / 2f
        val axis = h / 2f
        val reach = if (full) h * 0.31f else h / 2f - 1f
        val smooth = frame.bars
        val peaks = frame.peaks
        // Level is information: a breakdown is a row of small beads, not the drop
        // renormalised to the same height.
        val loud = 0.3f + 0.7f * clamp(frame.level * 1.5f * (0.7f + 0.3f * intensity), 0f, 1f)
        val breathe = !reduced && !eco

        for (i in 0 until bars) {
            val t = if (bars == 1) 0f else i.toFloat() / (bars - 1)
            val v = smooth[i] * loud
            // The bass is a lozenge, the treble a needle; a loud band swells.
            val taper = 1.06f - 0.26f * t
            val weight = max(0f, 1f - t * 2.5f) * min(1f, v * 1.4f)
            val halfW = min(
                r * taper * (0.88f + 0.22f * min(1f, v / 0.9f) + 0.18f * weight),
                bw * 0.46f,
            )
            val breath = if (breathe) sin(TAU * (beats / BREATH_BEATS + t * 0.55f)) else 0f
            val rest = 1f - 0.65f * clamp((v - 0.05f) / 0.65f, 0f, 1f)
            val minHalf = halfW * (1f + 0.46f * (0.5f + 0.5f * breath) * rest)
            val half = max(minHalf, v * reach)
            val x = i * bw + (bw - halfW * 2f) / 2f
            if (flat) {
                scope.drawRoundRect(
                    color = themeColour(frame, t),
                    topLeft = Offset(x, axis - half),
                    size = Size(halfW * 2f, half * 2f),
                    cornerRadius = androidx.compose.ui.geometry.CornerRadius(halfW, halfW),
                )
                continue
            }
            // The band's colour: the sleeve's zone under this position when the
            // walk is live, else the palette's own low → high walk.
            val zone = colourAt(frame, t)
            val ph = max(minHalf, (if (i < peaks.size) peaks[i] else 0f) * reach * loud)
            if (full && !eco && ph > half + 1.5f) {
                scope.drawRoundRect(
                    // On cream, a 16 % ghost of a LIGHT value is a pastel slab
                    // (the first capture showed a pink halo): the ink theme gives
                    // the afterglow an ink value, as bars.js does for its body.
                    color = if (ink) {
                        // A deeper, fainter ink: at 18 % of a mid value the ghost
                        // read as a second, paler row (the Papier capture).
                        hsl(zone.hue, zone.sat * 0.7f, 0.30f, 0.10f)
                    } else {
                        hsl(zone.hue + 12f, zone.sat * 0.9f, zone.light, 0.16f)
                    },
                    topLeft = Offset(x, axis - ph),
                    size = Size(halfW * 2f, ph * 2f),
                    cornerRadius = androidx.compose.ui.geometry.CornerRadius(halfW, halfW),
                )
            }
            // The capsule's own gradient, in two fills: a warm rim, then the lit
            // body inside it (flat colours on purpose — a gradient object per bar
            // per frame is garbage the fallback cannot afford).
            scope.drawRoundRect(
                color = if (ink) {
                    hsl(zone.hue + 8f, zone.sat, 0.30f, 0.95f)
                } else {
                    hsl(zone.hue + 14f, zone.sat, min(0.9f, zone.light - 0.01f), 0.92f)
                },
                topLeft = Offset(x, axis - half),
                size = Size(halfW * 2f, half * 2f),
                cornerRadius = androidx.compose.ui.geometry.CornerRadius(halfW, halfW),
            )
            if (eco) continue
            val inset = min(1.4f, halfW * 0.3f)
            val iw = halfW * 2f - inset * 2f
            val ih = half * 2f - inset * 2f
            if (iw > 0.6f && ih > 0.6f) {
                scope.drawRoundRect(
                    color = if (ink) {
                        hsl(zone.hue, zone.sat * 0.75f, 0.44f, 0.92f)
                    } else {
                        hsl(
                            zone.hue,
                            zone.sat * 0.85f,
                            min(0.92f, zone.light + 0.10f + 0.22f * v),
                            0.74f + 0.26f * min(1f, v / 0.6f),
                        )
                    },
                    topLeft = Offset(x + inset, axis - half + inset),
                    size = Size(iw, ih),
                    cornerRadius = androidx.compose.ui.geometry.CornerRadius(
                        max(0.5f, halfW - inset),
                        max(0.5f, halfW - inset),
                    ),
                )
            }
            // A pale bead on the axis — the hot core, not a specular stripe.
            val coreH = max(1f, min(half, halfW * 1.7f))
            scope.drawRoundRect(
                color = if (ink) {
                    hsl(zone.hue, zone.sat * 0.45f, 0.76f, 0.3f + 0.25f * min(1f, v / 0.6f))
                } else {
                    hsl(zone.hue, zone.sat * 0.3f, 0.9f, 0.2f + 0.3f * min(1f, v / 0.6f))
                },
                topLeft = Offset(x + inset, axis - coreH),
                size = Size(max(0.6f, halfW * 2f - inset * 2f), coreH * 2f),
                cornerRadius = androidx.compose.ui.geometry.CornerRadius(
                    max(0.5f, halfW - inset),
                    max(0.5f, halfW - inset),
                ),
            )
        }
    }

    // --- the oscilloscope ----------------------------------------------------

    /** One lane's geometry, as `scope.js#layout` builds it. */
    private class Lane(
        val horizontal: Boolean,
        val ox: Float,
        val oy: Float,
        val ax: Float,
        val ay: Float,
        val vx: Float,
        val vy: Float,
        val len: Float,
        val span: Float,
    )

    /**
     * `scope.js#draw`, for one frame. `wash` is the phosphor's per-frame alpha
     * (`1 - exp(-dt / PERSIST)`, clamped), computed by the caller from the paint
     * interval — a time constant, not a per-frame constant, so the exposure is
     * the same at 30 and at 60.
     */
    fun scope(
        scope: DrawScope,
        frame: VizFrame,
        intensity: Float,
        orientation: String,
        colour: String,
        eco: Boolean,
        wash: Float,
        measurer: TextMeasurer,
        labelSize: Float,
    ) {
        val cols = frame.scopeA.size / 2
        if (cols == 0) return
        val w = scope.size.width
        val h = scope.size.height
        if (w <= 0f || h <= 0f) return
        val lanes = lanes(w, h, orientation)
        // `glow` is the tier's HALO budget (0.7..1.4 in the SPA); a phone takes
        // the medium tier's, and eco takes the floor.
        // `glow` is the tier's HALO budget and `passes` its stroke count: the SPA
        // draws the middle BODY stroke only at `passes > 2` (high and ultra). A
        // phone is the medium tier, so the beam is a halo and a core — which is
        // also two paths per lane instead of three, measured on the emulator.
        val glow = if (eco) 0.7f else 1.0f
        val scale = frame.gain * DEFLECT * (0.55f + clamp(intensity, 0f, 1f) * 0.45f)
        val hue = frame.hue.toFloat()
        val sat = frame.sat.toFloat()
        val spread = frame.spread.toFloat()

        val halo = colorOf(colour, hue, sat, spread, 0, 0.5f)

        val core = colorOf(colour, hue, sat, spread, 0, 0.88f)
        val haloB = colorOf(colour, hue, sat, spread, 1, 0.5f)

        val coreB = colorOf(colour, hue, sat, spread, 1, 0.88f)

        lanes.forEachIndexed { channel, lane ->
            if (lane.len <= 2f || lane.span <= 1f) return@forEachIndexed
            // The instrument's face is not light either: a graticule you can read
            // on a desktop and cannot on a phone is the same fault as the beam.
            graticule(scope, lane, hue, sat, wash * 2.1f)
            marker(scope, lane, hue, wash * 1.6f * frame.locked)
            label(
                scope, lane, if (channel == 0) "G" else "D",
                hue, sat, spread, wash * 1.4f, measurer, labelSize,
            )
        }

        val coarse = clamp(sqrt(max(1f, scope.size.width) / max(1f, cols.toFloat())), 1f, 2.2f)
        val coreW = clamp(min(lanes[0].span, lanes[1].span) * 0.013f, 1f, 2.2f) * coarse
        // ONE PATH per channel per frame, stroked twice (halo, then beam). The
        // first version rebuilt the same 500-segment path for every stroke — four
        // builds a frame instead of two — and the CI runner's software rasteriser
        // blew the 16.7 ms budget on the oscilloscope alone.
        val path = Path()
        lanes.forEachIndexed { channel, lane ->
            if (lane.len <= 2f || lane.span <= 1f) return@forEachIndexed
            val trace = if (channel == 0) frame.scopeA else frame.scopeB
            tracePath(path, lane, trace, cols, scale)
            // The halo is what makes a one-pixel stroke read as light rather than
            // as ink. Eco keeps the beam and drops the halo (the flat grade), and
            // the halo is 2.4 beams wide rather than 3.4: a fat antialiased stroke
            // is the most expensive thing a software rasteriser draws.
            if (!eco) {
                strokePath(
                    scope, path,
                    if (channel == 0) halo else haloB,
                    coreW * 2.4f, wash * HALO_BRIGHT * glow,
                )
            }
            strokePath(
                scope, path,
                if (channel == 0) core else coreB,
                coreW, wash * CORE_BRIGHT,
            )
        }
    }

    private fun lanes(w: Float, h: Float, orientation: String): List<Lane> {
        val horizontal = orientation != "vertical"
        val a: FloatArray
        val b: FloatArray
        if (horizontal) {
            // Two bands stacked: the top half and the bottom half, no occluder on
            // a phone (the caption strip is not the artwork's box).
            a = floatArrayOf(0f, 0f, w, h / 2f)
            b = floatArrayOf(0f, h / 2f, w, h / 2f)
        } else {
            a = floatArrayOf(0f, 0f, w / 2f, h)
            b = floatArrayOf(w / 2f, 0f, w / 2f, h)
        }
        return listOf(a, b).map { r ->
            val (x, y, rw, rh) = r
            val short = min(rw, rh)
            // Never more than a third of the short side: a lane a few pixels tall
            // would otherwise invert its own inside and draw back to front.
            val pad = min(clamp(short * 0.09f, 4f, 26f), short * 0.35f)
            val inX = rw - 2f * pad
            val inY = rh - 2f * pad
            if (horizontal) {
                Lane(true, x + pad, y + rh / 2f, inX, 0f, 0f, -inY / 2f, inX, inY / 2f)
            } else {
                Lane(false, x + rw / 2f, y + pad, 0f, inY, inX / 2f, 0f, inY, inX / 2f)
            }
        }
    }

    /** `scope.js#strokeLane`'s geometry: max then min per column, so a dense column draws its true extent. */
    private fun tracePath(
        path: Path,
        lane: Lane,
        trace: List<Float>,
        cols: Int,
        scale: Float,
    ) {
        path.reset()
        for (c in 0 until cols) {
            val t = if (cols == 1) 0f else c.toFloat() / (cols - 1)
            val bx = lane.ox + lane.ax * t
            val by = lane.oy + lane.ay * t
            val hi = clamp(trace.getOrElse(c * 2 + 1) { 0f } * scale, -1f, 1f)
            val lo = clamp(trace.getOrElse(c * 2) { 0f } * scale, -1f, 1f)
            val x1 = bx + lane.vx * hi
            val y1 = by + lane.vy * hi
            if (c == 0) path.moveTo(x1, y1) else path.lineTo(x1, y1)
            if (lo != hi) path.lineTo(bx + lane.vx * lo, by + lane.vy * lo)
        }
    }

    /** One stroke of a path already built: the halo and the beam share it. */
    private fun strokePath(
        scope: DrawScope,
        path: Path,
        colour: Color,
        width: Float,
        alpha: Float,
    ) {
        if (alpha <= 0.002f) return
        scope.drawPath(
            path = path,
            color = colour.copy(alpha = clamp(alpha, 0f, 1f)),
            style = Stroke(width = width, cap = StrokeCap.Round, join = StrokeJoin.Round),
            blendMode = androidx.compose.ui.graphics.BlendMode.Plus,
        )
    }

    /** `scope.js#graticule`: the frame, the halves, then the divisions, three weights. */
    private fun graticule(scope: DrawScope, lane: Lane, hue: Float, sat: Float, a: Float) {
        val frameColour = hsl(hue, sat * 0.22f, 0.64f, a * 0.55f)
        val path = Path()
        path.moveTo(lane.ox + lane.vx, lane.oy + lane.vy)
        path.lineTo(lane.ox + lane.ax + lane.vx, lane.oy + lane.ay + lane.vy)
        path.lineTo(lane.ox + lane.ax - lane.vx, lane.oy + lane.ay - lane.vy)
        path.lineTo(lane.ox - lane.vx, lane.oy - lane.vy)
        path.close()
        scope.drawPath(path, frameColour, style = Stroke(width = 1.2f))
        val line = Path()
        val grid = hsl(hue, sat * 0.18f, 0.58f, a * 0.28f)
        fun segment(t0: Float, v0: Float, t1: Float, v1: Float) {
            line.moveTo(
                lane.ox + lane.ax * t0 + lane.vx * v0,
                lane.oy + lane.ay * t0 + lane.vy * v0,
            )
            line.lineTo(
                lane.ox + lane.ax * t1 + lane.vx * v1,
                lane.oy + lane.ay * t1 + lane.vy * v1,
            )
        }
        segment(0f, 0.5f, 1f, 0.5f)
        segment(0f, -0.5f, 1f, -0.5f)
        for (d in 1 until DIVISIONS) {
            val t = d.toFloat() / DIVISIONS
            segment(t, -1f, t, 1f)
            segment(t, -1f, t, -0.94f)
            segment(t, 0.94f, t, 1f)
        }
        scope.drawPath(line, grid, style = Stroke(width = 1f))
        // Ticks on the frame at the half lines, then the zero line — brightest,
        // because it is the one the eye reads the wave against.
        val ticks = Path()
        for (v in floatArrayOf(-0.5f, 0f, 0.5f)) {
            ticks.moveTo(lane.ox + lane.vx * v, lane.oy + lane.vy * v)
            ticks.lineTo(
                lane.ox + lane.ax * 0.035f + lane.vx * v,
                lane.oy + lane.ay * 0.035f + lane.vy * v,
            )
            ticks.moveTo(lane.ox + lane.ax * 0.965f + lane.vx * v, lane.oy + lane.ay * 0.965f + lane.vy * v)
            ticks.lineTo(lane.ox + lane.ax + lane.vx * v, lane.oy + lane.ay + lane.vy * v)
        }
        scope.drawPath(ticks, hsl(hue, sat * 0.22f, 0.62f, a * 0.5f), style = Stroke(width = 1f))
        val zero = Path()
        zero.moveTo(lane.ox, lane.oy)
        zero.lineTo(lane.ox + lane.ax, lane.oy + lane.ay)
        scope.drawPath(zero, hsl(hue, sat * 0.35f, 0.74f, a * 0.9f), style = Stroke(width = 1f))
    }

    /** `scope.js#marker`: the trigger's own marker, faded by the lock. */
    private fun marker(scope: DrawScope, lane: Lane, hue: Float, a: Float) {
        if (a <= 0.02f) return
        val s = clamp(lane.span * 0.07f, 3.5f, 9f)
        val path = Path()
        if (lane.horizontal) {
            path.moveTo(lane.ox - s * 1.4f, lane.oy - s)
            path.lineTo(lane.ox - s * 1.4f, lane.oy + s)
        } else {
            path.moveTo(lane.ox - s, lane.oy - s * 1.4f)
            path.lineTo(lane.ox + s, lane.oy - s * 1.4f)
        }
        path.lineTo(lane.ox, lane.oy)
        path.close()
        scope.drawPath(path, hsl(hue + 18f, 0.5f, 0.8f, a))
    }

    /** `scope.js#label`: G/D with their own dot — an instrument, not a decoration. */
    private fun label(
        scope: DrawScope,
        lane: Lane,
        text: String,
        hue: Float,
        sat: Float,
        spread: Float,
        a: Float,
        measurer: TextMeasurer,
        size: Float,
    ) {
        if (a <= 0.02f) return
        val fontSize = clamp(min(lane.span, lane.len) * 0.20f, 10f, 16f).coerceAtMost(size)
        val x = lane.ox + if (lane.horizontal) lane.len * 0.012f else -lane.span * 0.88f
        val y = if (lane.horizontal) lane.oy - lane.span * 0.88f else lane.oy + lane.len * 0.012f
        val dotHue = if (text == "G") hue - spread * 0.5f else hue + spread * 0.5f
        scope.drawCircle(
            color = hsl(dotHue, sat, 0.72f, a * 0.9f),
            radius = fontSize * 0.16f,
            center = Offset(x + fontSize * 0.2f, y + fontSize * 0.55f),
        )
        val layout = measurer.measure(
            text = text,
            style = TextStyle(fontSize = fontSize.sp, color = hsl(hue, sat * 0.25f, 0.82f, a)),
        )
        scope.drawText(layout, topLeft = Offset(x + fontSize * 0.9f, y))
    }

    // --- the palette, as the scene paints it ---------------------------------

    private data class Hsl(val hue: Float, val sat: Float, val light: Float)

    /**
     * `palette.js#colourAt` over the seven stops the frame carries: the short way
     * round the wheel, so 350° → 10° crosses red instead of sweeping the wheel
     * backwards. Falls back to the low → high walk when the sleeve has no zones.
     */
    private fun colourAt(frame: VizFrame, t: Float): Hsl {
        val stops = frame.zones
        if (frame.hasZones && stops.size >= 6) {
            val count = stops.size / 3
            val x = clamp(t, 0f, 1f) * (count - 1)
            val i = min(count - 2, x.toInt())
            val k = x - i
            val a = i * 3
            val b = a + 3
            val d = ((stops[b] - stops[a]) % 360f + 540f) % 360f - 180f
            return Hsl(
                hue = ((stops[a] + d * k) % 360f + 360f) % 360f,
                sat = stops[a + 1] + (stops[b + 1] - stops[a + 1]) * k,
                light = stops[a + 2] + (stops[b + 2] - stops[a + 2]) * k,
            )
        }
        val hue = frame.hue.toFloat() - frame.spread.toFloat() * 0.5f + frame.spread.toFloat() * t
        return Hsl(hue, frame.sat.toFloat(), frame.light.toFloat())
    }

    /** The strip's one flat colour: the theme's accent, walked for a hue. */
    private fun themeColour(frame: VizFrame, t: Float): Color {
        val hue = frame.hue.toFloat() - frame.spread.toFloat() * 0.5f + frame.spread.toFloat() * t
        return hsl(hue, frame.sat.toFloat(), frame.light.toFloat(), 1f)
    }

    /** `scope.js#hueFor` + `#strokeStyleFor`: duo, mono — `sweep` is folded to duo here. */
    private fun colorOf(
        colour: String,
        hue: Float,
        sat: Float,
        spread: Float,
        channel: Int,
        light: Float,
    ): Color {
        val chosen = when (colour) {
            "mono" -> hue
            else -> if (channel == 0) hue - spread * 0.5f else hue + spread * 0.5f
        }
        return hsl(chosen, sat, light, 1f)
    }

    /** `util.js#hsl`, in Compose's own HSL constructor (hue in degrees). */
    fun hsl(hue: Float, sat: Float, light: Float, alpha: Float = 1f): Color {
        val h = ((hue % 360f) + 360f) % 360f
        return Color.hsl(h, clamp(sat, 0f, 1f), clamp(light, 0f, 1f), clamp(alpha, 0f, 1f))
    }

    private fun clamp(v: Float, lo: Float, hi: Float): Float = if (v < lo) lo else if (v > hi) hi else v

    /** `scope.js#draw`'s own `wash`, from the paint interval. */
    fun wash(dtSeconds: Float): Float = 1f - exp(-max(0.001f, dtSeconds) / PERSIST)

    /** Rounds a hue to a whole degree, for the diagnostic line and the tests. */
    fun degrees(hue: Float): Int = ((hue % 360f + 360f) % 360f).roundToInt()

    /** The scope's beam width, exposed so a test can hold the tapers' numbers. */
    fun beamWidth(span: Float, cols: Int, width: Float): Float {
        val coarse = clamp(sqrt(max(1f, width) / max(1f, cols.toFloat())), 1f, 2.2f)
        return clamp(span * 0.013f, 1f, 2.2f) * coarse
    }

    /** Whether two colours are the same, for the tests' own assertions. */
    fun sameHue(a: Float, b: Float, tolerance: Float): Boolean {
        val d = abs(((b - a) % 360f + 540f) % 360f - 180f)
        return d <= tolerance
    }

    /** A brush for a caller that wants the palette's walk outside this object. */
    fun walkBrush(frame: VizFrame, start: Offset, end: Offset): Brush = Brush.linearGradient(
        listOf(
            hsl(frame.hue.toFloat() - frame.spread.toFloat() * 0.5f, frame.sat.toFloat(), frame.light.toFloat()),
            hsl(frame.hue.toFloat(), frame.sat.toFloat(), frame.light.toFloat()),
            hsl(frame.hue.toFloat() + frame.spread.toFloat() * 0.5f, frame.sat.toFloat(), frame.light.toFloat()),
        ),
        start = start,
        end = end,
    )
}
