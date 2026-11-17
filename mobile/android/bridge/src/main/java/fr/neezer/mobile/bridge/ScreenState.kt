package fr.neezer.mobile.bridge

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.channelFlow

/**
 * What a screen shows while its data arrives (rule 3).
 *
 * The three states come straight from the plan: the cached copy first (so a
 * screen paints at once), the fresh answer replacing it, and — when the network
 * fails — the cached copy KEPT and flagged [Ready.stale] ("Hors ligne · version
 * mémorisée"). With no cached copy there is [Failed], with the cause and a
 * working retry, never an endless spinner.
 */
sealed interface ScreenState<out T> {
    /** Nothing to draw yet. */
    data object Loading : ScreenState<Nothing>

    /**
     * Something to draw. [stale] is true while what is shown is the copy kept on
     * the device (offline, or a refresh that failed).
     */
    data class Ready<T>(val data: T, val stale: Boolean = false) : ScreenState<T>

    /** Nothing to draw and a failure to name. [retry] collects the read again. */
    data class Failed(val cause: String, val retry: () -> Unit) : ScreenState<Nothing>
}

/**
 * The one way a screen loads (docs/agents/mobile.md, MB2's contract): [cached]
 * is disk-only, [fresh] goes to the network on the `User` lane.
 *
 * - cached, then fresh → `Loading`, `Ready(stale = true)`, `Ready(stale = false)`
 * - fresh only → `Loading`, `Ready(stale = false)`
 * - fresh fails with a cached copy → the copy stays, `Ready(stale = true)`
 * - fresh fails with nothing → `Failed(cause, retry)`, and [ScreenState.Failed.retry]
 *   re-runs the read (one click = one attempt, `webapp/test/failure/run.mjs`'s
 *   rule; nothing retries by itself)
 *
 * Two details that cost a debugging session each, and are why this file has its
 * own JVM test:
 *
 * - **A retry that arrives before the flow listens is NOT lost.** The signal is
 *   a CONFLATED channel, not a `SharedFlow`: with no replay, a `tryEmit` that
 *   lands between the `Failed` value and the next `receive()` would be dropped,
 *   and the screen would sit on "Réessayer" forever. Measured by the test where
 *   the collector calls `retry()` inside the same `collect`.
 * - **A screen that is closed stops the work.** The flow is a `channelFlow`, so
 *   leaving the screen cancels it, which cancels the `suspend` FFI call — and
 *   Rust drops the future before it writes anything (cancellation-safe).
 */
fun <T> screenFlow(
    cached: suspend () -> T?,
    fresh: suspend () -> T,
): Flow<ScreenState<T>> = channelFlow {
    // CONFLATED: a second click while the first is in flight is the same click.
    val retries = Channel<Unit>(Channel.CONFLATED)
    var announced = false

    while (true) {
        if (!announced) {
            send(ScreenState.Loading)
            announced = true
        }

        val kept: T? = try {
            cached()
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
            // A cache that cannot be read is not a failure the person must see:
            // the fresh read decides.
            null
        }
        if (kept != null) {
            send(ScreenState.Ready(kept, stale = true))
        }

        try {
            send(ScreenState.Ready(fresh(), stale = false))
            return@channelFlow
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            if (kept != null) {
                // The remembered version is what the person keeps; the screen
                // says so instead of emptying itself.
                send(ScreenState.Ready(kept, stale = true))
                return@channelFlow
            }
            send(ScreenState.Failed(NzErrors.text(e)) { retries.trySend(Unit) })
            retries.receive()
        }
    }
}
