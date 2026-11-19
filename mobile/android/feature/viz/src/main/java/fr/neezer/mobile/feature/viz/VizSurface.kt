package fr.neezer.mobile.feature.viz

import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.unit.IntSize
import fr.neezer.mobile.designsystem.Sillon
import kotlinx.coroutines.isActive
import uniffi.nz_ffi.VizFrame

/**
 * The visualiser's drawing surface (lot MB21): bars or the oscilloscope, fed by
 * `VizApi::frame()` — `components/Visualizer.svelte`'s host, ported.
 *
 * The host owns what the scenes should not have to care about: the frame loop and
 * its ceiling, the analysis subscription's life, the shape the surface asks for,
 * and the theme's ink. The scene draws (see [VizDraw]).
 *
 * # The loop, and rule 1
 *
 * - **Analysis runs only while this composable is on screen.** `setVisible` is
 *   called from a `DisposableEffect`'s body and its `onDispose` — never on a
 *   timer — and the Rust side holds no tap subscription when the last view
 *   leaves. That is the acceptance's "0 analysis tick while no view is visible".
 * - **Painting is capped, analysis is not.** The loop waits on
 *   `withFrameNanos` and skips a frame when the last paint is younger than the
 *   person's cap (30 or 60), so a 120 Hz panel costs 60 paints and the analyser
 *   still runs at its own ~94 Hz: a beat is never missed because the screen was
 *   capped.
 * - **A paused player stops the loop**, as `Visualizer.svelte#applyPaused` does:
 *   the last frame fades out over the motion token and no `frame()` call is made
 *   while nothing plays.
 * - **Eco draws nothing at all** (`mode == "off"` comes from the settings, which
 *   resolve eco to `off`): the screen shows the notice instead.
 *
 * # The shape
 *
 * `bars.js#resize`: one capsule per 16 dp in the player's strip, one per 20.5 dp
 * full screen, clamped 18..96 — and one scope column per dp, clamped to 16..512
 * (`scope.js#layout`'s own ceiling). It is sent on a SIZE or MODE change only:
 * changing either resets an envelope in `nz-viz`, which is right on a rotation
 * and wrong every frame.
 *
 * @param mode `bars` or `scope` (the settings' effective mode).
 * @param layout `full` on the visualiser screen, `strip` in the player.
 */
@Composable
fun VizSurface(
    mode: String,
    modifier: Modifier = Modifier,
    service: VizService? = VizCore.service(),
    intensity: Float = 0.7f,
    scopeOrientation: String = "horizontal",
    scopeColour: String = "duo",
    fps: Int = 60,
    playing: Boolean = true,
    layout: String = "full",
) {
    val colours = Sillon.colors
    val motion = Sillon.motion
    val density = LocalDensity.current
    val measurer = rememberTextMeasurer()
    val light = !colours.isDark
    var frame by remember { mutableStateOf<VizFrame?>(null) }
    var size by remember { mutableStateOf(IntSize.Zero) }
    // THE BACKING STORE IS COARSER THAN THE DISPLAY, exactly as `Visualizer.svelte`
    // caps `preset.dpr` and its total pixel count: a scene is not text, and the
    // emulator's software rasteriser fills 1080 × 2400 pixels at a measured
    // 26.9 ms p95 for the bars and 50.1 ms for the scope (this lot's first device
    // run). Rendering into a bitmap at twice the dp size and blitting it up is
    // 2.25× fewer pixels at 2.75× density, and the picture stays a light show.
    val backing = remember { mutableStateOf<androidx.compose.ui.graphics.ImageBitmap?>(null) }
    val fontScale = density.fontScale
    var beats by remember { mutableFloatStateOf(0f) }
    var wash by remember { mutableFloatStateOf(VizDraw.wash(1f / 60f)) }
    val full = layout != "strip"

    // The shape: capsules and columns from the width the surface really has.
    val widthDp = with(density) { size.width.toDp().value }
    LaunchedEffect(service, widthDp, mode, full) {
        if (service == null || widthDp <= 0f) return@LaunchedEffect
        val bars = (widthDp / if (full) 20.5f else 16f).toInt().coerceIn(18, 96)
        val cols = widthDp.toInt().coerceIn(16, 512)
        service.setShape(bars, cols)
    }

    // The analysis is ON only while this surface is: start and dispose, no timer.
    DisposableEffect(service, mode) {
        if (service != null && mode != "off") service.setVisible(mode, true)
        onDispose { if (service != null) service.setVisible(mode, false) }
    }

    LaunchedEffect(service, mode, fps, playing) {
        if (service == null || mode == "off" || !playing) return@LaunchedEffect
        val gap = if (fps > 0) 1_000_000_000L / fps else 0L
        var lastPaint = 0L
        var lastNanos = 0L
        while (isActive) {
            withFrameNanos { now ->
                // A couple of milliseconds of slack: at a 60 cap on a 60 Hz
                // display an exact comparison drops every other frame whenever a
                // vsync runs 0.1 ms early (`Visualizer.svelte#paint`).
                if (gap == 0L || now - lastPaint >= gap - 2_000_000L) {
                    val dt = if (lastNanos == 0L) 0f else (now - lastNanos) / 1_000_000_000f
                    lastNanos = now
                    lastPaint = now
                    wash = VizDraw.wash(dt.coerceIn(0.001f, 0.25f))
                    // No tempo at the spectrum depth: the SPA's own fallback
                    // period (0.5 s) advances the rest breath.
                    beats += dt / 0.5f
                    frame = service.frame()
                }
            }
        }
    }

    // A paused player fades the picture out over the motion token and the shell
    // draws the notice over it (`applyPaused`: dimmed, then idle).
    val alpha by animateFloatAsState(
        targetValue = if (playing) 1f else 0f,
        animationSpec = if (motion.moves) motion.base() else motion.tween(0),
        label = "viz-fade",
    )

    // The sprite the scene is rendered into, re-created on a size change: its
    // scale is `min(1, 2 * dp / px)`, never below half.
    val renderScale = run {
        val px = size.width
        if (px <= 0) 1f else (2f * with(density) { px.toDp().value } / px).coerceIn(0.5f, 1f)
    }
    val sprite = if (size.width > 0 && size.height > 0) {
        val w = maxOf(1, (size.width * renderScale).toInt())
        val h = maxOf(1, (size.height * renderScale).toInt())
        val current = backing.value
        if (current == null || current.width != w || current.height != h) {
            androidx.compose.ui.graphics.ImageBitmap(w, h).also { backing.value = it }
        } else {
            current
        }
    } else {
        null
    }

    Box(modifier) {
        Canvas(
            modifier = Modifier
                .fillMaxSize()
                .onSizeChanged { size = it }
                // Decorative, like the SPA's `aria-hidden` canvases: a screen
                // reader announces the card and the controls, never a waveform.
                .clearAndSetSemantics { },
        ) {
            val current = frame ?: return@Canvas
            if (alpha <= 0.01f && !playing) return@Canvas
            val target = sprite
            if (target != null) {
                // Drawn into the coarse backing store, then blitted once: one
                // `CanvasDrawScope` per paint, no layer, no extra allocation.
                val canvas = androidx.compose.ui.graphics.Canvas(target)
                androidx.compose.ui.graphics.drawscope.CanvasDrawScope().draw(
                    density,
                    androidx.compose.ui.unit.LayoutDirection.Ltr,
                    canvas,
                    androidx.compose.ui.geometry.Size(target.width.toFloat(), target.height.toFloat()),
                ) {
                    drawScene(
                        current, mode, intensity, scopeOrientation, scopeColour,
                        light, full, motion, wash, alpha, beats, measurer, fontScale,
                    )
                }
                drawImage(
                    image = target,
                    srcOffset = androidx.compose.ui.unit.IntOffset.Zero,
                    srcSize = androidx.compose.ui.unit.IntSize(target.width, target.height),
                    dstOffset = androidx.compose.ui.unit.IntOffset.Zero,
                    dstSize = androidx.compose.ui.unit.IntSize(size.width, size.height),
                    filterQuality = androidx.compose.ui.graphics.FilterQuality.Low,
                )
            } else {
                drawScene(
                    current, mode, intensity, scopeOrientation, scopeColour,
                    light, full, motion, wash, alpha, beats, measurer, fontScale,
                )
            }
        }
    }
}


/**
 * The scene itself — the scope's face and traces, or the row of capsules — in
 * WHATEVER `DrawScope` it is handed: the screen's own canvas, the coarse backing
 * store it renders into, or a still. `VizStill` calls the same function, so what
 * the committed captures show is what a device paints.
 */
private fun androidx.compose.ui.graphics.drawscope.DrawScope.drawScene(
    current: VizFrame,
    mode: String,
    intensity: Float,
    scopeOrientation: String,
    scopeColour: String,
    light: Boolean,
    full: Boolean,
    motion: fr.neezer.mobile.designsystem.NzMotion,
    wash: Float,
    alpha: Float,
    beats: Float,
    measurer: androidx.compose.ui.text.TextMeasurer,
    fontScale: Float,
) {
    if (mode == "scope") {
        // The scope's smoked glass: OPAQUE and dark on every theme — an
        // instrument's face, and the one surface that is not the page's. A
        // translucent near-black wash over cream rendered mid-grey and took the
        // trace with it (the first Papier capture).
        //
        // ONE FLAT FILL, not the SPA's radial gradient. At 6 % lightness the two
        // are the same picture, and a full-canvas radial gradient is per-pixel
        // maths the software rasteriser pays for on every frame: it was the
        // oscilloscope's cost on the CI runner, where the scene's p95 sat just over
        // 16.7 ms. Measured on the emulator: the scope went from 8.80 to 3.9 ms p95.
        drawRect(color = VizDraw.hsl(current.hue.toFloat(), current.sat.toFloat() * 0.35f, 0.06f, 1f))
        VizDraw.scope(
            scope = this,
            frame = current,
            intensity = intensity,
            orientation = scopeOrientation,
            colour = scopeColour,
            eco = !motion.moves,
            wash = wash * alpha,
            measurer = measurer,
            // The label's font follows the theme's scale, so 200 % text does not
            // clip the G/D of a fixed-size instrument face.
            labelSize = 16f * fontScale,
        )
    } else {
        VizDraw.bars(
            scope = this,
            frame = current,
            intensity = intensity,
            full = full,
            light = light,
            eco = !motion.moves,
            reduced = !motion.moves,
            beats = beats,
        )
    }
}

/**
 * The scene with a frame handed in, for a screenshot or a preview: no service, no
 * loop, no FFI — the pixels only. The screen tests and the design-system bench
 * use it, and so does a caller that already has a frame.
 */
@Composable
fun VizStill(
    frame: VizFrame,
    mode: String,
    modifier: Modifier = Modifier,
    intensity: Float = 0.7f,
    scopeOrientation: String = "horizontal",
    scopeColour: String = "duo",
    layout: String = "full",
) {
    val colours = Sillon.colors
    val motion = Sillon.motion
    val density = LocalDensity.current
    val measurer = rememberTextMeasurer()
    val full = layout != "strip"
    Canvas(
        modifier = modifier.fillMaxSize().clearAndSetSemantics { },
    ) {
        if (mode == "scope") {
            // The same flat smoked glass the screen draws (`drawScene`), not the
            // SPA's radial gradient: at 6 % lightness the picture is the same and
            // the per-pixel maths is not.
            drawRect(
                color = VizDraw.hsl(
                    frame.hue.toFloat(),
                    frame.sat.toFloat() * 0.35f,
                    0.06f,
                    1f,
                ),
            )
            VizDraw.scope(
                scope = this,
                frame = frame,
                intensity = intensity,
                orientation = scopeOrientation,
                colour = scopeColour,
                eco = false,
                wash = 1f,
                measurer = measurer,
                labelSize = 16f * density.fontScale,
            )
        } else {
            VizDraw.bars(
                scope = this,
                frame = frame,
                intensity = intensity,
                full = full,
                light = !colours.isDark,
                eco = false,
                reduced = !motion.moves,
                beats = 0f,
            )
        }
    }
}

/** The colour a scope's smoke is built on, for the callers that need a ground. */
internal fun vizGround(colour: String, frame: VizFrame): Color = when (colour) {
    "mono" -> VizDraw.hsl(frame.hue.toFloat(), frame.sat.toFloat(), 0.1f)
    else -> VizDraw.hsl(frame.hue.toFloat(), frame.sat.toFloat() * 0.5f, 0.08f)
}
