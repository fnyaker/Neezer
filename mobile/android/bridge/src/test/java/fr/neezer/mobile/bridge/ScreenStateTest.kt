package fr.neezer.mobile.bridge

import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * `screenFlow` is what every screen paints from (rule 3), so its four shapes are
 * held here as a JVM test: no device, no clock, no network. A test that changed
 * the KOTLIN behaviour would pass here long before a Compose test could see it.
 *
 * No `kotlinx-coroutines-test` is needed: a `CompletableDeferred` is the event
 * the assertions wait on (worker rule 6), and the `withTimeout` is a deadlock
 * guard, not a delay.
 */
class ScreenStateTest {
    /** Collects the flow in its own scope and answers when [until] holds. */
    private class Collector<T>(private val until: (ScreenState<T>) -> Boolean) {
        val states = mutableListOf<ScreenState<T>>()
        private val done = CompletableDeferred<Unit>()
        private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)

        fun collect(flow: kotlinx.coroutines.flow.Flow<ScreenState<T>>): kotlinx.coroutines.Job =
            scope.launch {
                flow.collect { state ->
                    states += state
                    if (until(state) && !done.isCompleted) done.complete(Unit)
                }
            }

        suspend fun await() = withTimeout(5_000) { done.await() }
    }

    /** cached, then fresh: Loading, the remembered copy, then the answer. */
    @Test
    fun a_cached_copy_paints_before_the_network_answers() = runBlocking {
        val collector = Collector<String> { it is ScreenState.Ready<*> && !it.stale }
        val job = collector.collect(
            screenFlow(cached = { "version mémorisée" }, fresh = { "version fraîche" }),
        )
        collector.await()
        job.cancel()
        assertEquals(
            listOf(
                ScreenState.Loading,
                ScreenState.Ready("version mémorisée", stale = true),
                ScreenState.Ready("version fraîche", stale = false),
            ),
            collector.states,
        )
    }

    /** No cache: Loading, then the answer. */
    @Test
    fun with_nothing_cached_the_first_paint_is_the_answer() = runBlocking {
        val collector = Collector<String> { it is ScreenState.Ready<*> }
        val job = collector.collect(screenFlow(cached = { null }, fresh = { "frais" }))
        collector.await()
        job.cancel()
        assertEquals(
            listOf(ScreenState.Loading, ScreenState.Ready("frais", stale = false)),
            collector.states,
        )
    }

    /** A failed refresh KEEPS the remembered copy and says it is stale. */
    @Test
    fun a_failure_with_a_cached_copy_keeps_it_and_marks_it_stale() = runBlocking {
        val collector = Collector<String> { it is ScreenState.Ready<*> }
        val job = collector.collect(
            screenFlow<String>(
                cached = { "mémorisée" },
                fresh = { throw IllegalStateException("Hors ligne") },
            ),
        )
        collector.await()
        job.cancel()
        assertEquals(
            listOf(
                ScreenState.Loading,
                ScreenState.Ready("mémorisée", stale = true),
                ScreenState.Ready("mémorisée", stale = true),
            ),
            collector.states,
        )
    }

    /**
     * Nothing cached and the network fails: `Failed` names the cause, and the
     * retry it hands the screen really collects the read again (one click, one
     * attempt — nothing retries by itself).
     */
    @Test
    fun with_nothing_cached_a_failure_names_the_cause_and_retries() = runBlocking {
        val attempts = AtomicInteger(0)
        val states = mutableListOf<ScreenState<String>>()
        val done = CompletableDeferred<Unit>()
        val job = CoroutineScope(SupervisorJob() + Dispatchers.Default).launch {
            screenFlow<String>(
                cached = { null },
                fresh = {
                    if (attempts.incrementAndGet() == 1) {
                        throw IllegalStateException("Le serveur ne répond pas")
                    }
                    "enfin"
                },
            ).collect { state ->
                states += state
                if (state is ScreenState.Failed) state.retry()
                if (state is ScreenState.Ready<*> && !state.stale) done.complete(Unit)
            }
        }
        withTimeout(5_000) { done.await() }
        job.cancel()

        assertEquals("states: $states", 3, states.size)
        assertEquals(ScreenState.Loading, states[0])
        val failed = states[1] as ScreenState.Failed
        assertEquals("Le serveur ne répond pas", failed.cause)
        assertEquals(ScreenState.Ready("enfin", stale = false), states[2])
        assertEquals("one click, one attempt", 2, attempts.get())
    }

    /** A cache that cannot be read is not the person's failure. */
    @Test
    fun an_unreadable_cache_does_not_become_the_failure() = runBlocking {
        val collector = Collector<String> { it is ScreenState.Ready<*> }
        val job = collector.collect(
            screenFlow<String>(
                cached = { throw IllegalStateException("base de données verrouillée") },
                fresh = { "frais" },
            ),
        )
        collector.await()
        job.cancel()
        assertEquals(
            listOf(ScreenState.Loading, ScreenState.Ready("frais", stale = false)),
            collector.states,
        )
    }
}
