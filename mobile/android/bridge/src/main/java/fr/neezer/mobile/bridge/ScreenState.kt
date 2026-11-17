package fr.neezer.mobile.bridge

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.channelFlow
import kotlinx.coroutines.flow.first

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
 * MB2 owns this file and its JVM test; MB0 gave it the shape every feature lot
 * compiles against, so no lot invents a second loading state.
 */
fun <T> screenFlow(
    cached: suspend () -> T?,
    fresh: suspend () -> T,
): Flow<ScreenState<T>> = channelFlow {
    val retries = MutableSharedFlow<Unit>(extraBufferCapacity = 1)
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
            send(ScreenState.Failed(NzErrors.text(e)) { retries.tryEmit(Unit) })
            retries.first()
        }
    }
}
