package fr.neezer.mobile.feature.settings

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.cancel
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Réglages' rules, on the JVM: the counts (what a section asks), the four states
 * of a card (rule 3), the optimistic click and its rollback, and one request per
 * drag. No JNA, no `.so`, no device — the fake is [FakeSettingsService].
 */
@OptIn(ExperimentalCoroutinesApi::class)
class SettingsModelTest {
    /** Every scope a test built, cancelled when it ends (a screen's lifetime). */
    private val scopes = mutableListOf<CoroutineScope>()

    @After
    fun closeScreens() {
        scopes.forEach { it.cancel() }
        scopes.clear()
    }

    /**
     * The model over a scope of the TEST's own, on an unconfined dispatcher: a
     * tap runs to its first suspension in the same call, which is exactly what
     * the acceptance asks of an optimistic screen ("it answers at once"). The
     * scope is not the test's child — a section waiting on « Réessayer » is a
     * live flow, and `runTest` must not wait for it — so [closeScreens] ends it.
     */
    private fun TestScope.model(
        service: FakeSettingsService,
        toasts: MutableList<String> = mutableListOf(),
    ): SettingsModel {
        val dispatcher = UnconfinedTestDispatcher(testScheduler)
        val scope = CoroutineScope(dispatcher)
        scopes += scope
        return SettingsModel(
            service = service,
            scope = scope,
            io = dispatcher,
            toast = { toasts.add(it) },
        )
    }

    /**
     * The number the acceptance is held to: opening *Sources* asks the SPA's own
     * three routes — one each — and paints the device's copy BEFORE any of them
     * answers (so the frame is never blocked by the link).
     */
    @Test
    fun a_cold_sources_section_asks_three_routes_and_a_warm_one_paints_from_the_copy() = runTest {
        val service = FakeSettingsService()
        val model = model(service)
        model.open("sources")

        // The administrator's cards are the SPA's two more (`/settings` + the
        // provider's verdict): five in all, asked only once `/access` said this
        // person is an administrator. A member's three are the next test.
        assertEquals(
            setOf("sources", "accounts", "access", "serverSettings", "deezerStatus"),
            service.calls.toSet(),
        )
        assertEquals(5, service.calls.size)
        assertTrue(model.state.sources.data != null)
        assertTrue(model.state.accounts.data != null)
        assertTrue(model.state.access.data != null)

        // The second visit: the copy is there BEFORE the network answers, so the
        // card has something to draw in the first frame. The read is held at the
        // gate — an event, not a duration — so what is observed is the frame the
        // person sees while the link is still working.
        service.keepCopies()
        service.calls.clear()
        val readGate = CompletableDeferred<Unit>()
        service.readGate = readGate
        val second = model(service)
        second.open("sources")
        assertNotNull("the copy paints first", second.state.sources.data)
        assertTrue("and it is flagged as the device's copy", second.state.sources.stale)
        assertEquals("with the link still working", 1, service.calls.count { it == "sources" })
        readGate.complete(Unit)
    }

    /** A card that cannot reach the server keeps the copy and says why (rule 3). */
    @Test
    fun a_failure_keeps_the_copy_with_its_cause() = runTest {
        val service = FakeSettingsService()
        service.keepCopies()
        service.failures["sources"] = IllegalStateException("hors ligne")
        val model = model(service)
        model.open("sources")

        val card = model.state.sources
        assertNotNull("the copy is kept", card.data)
        assertTrue("and the card says it is the kept one", card.stale)
        assertNull("a kept copy is not a failure to shout about", card.failure)
    }

    /** With no copy at all, the card names its cause and offers « Réessayer ». */
    @Test
    fun without_a_copy_the_cause_is_named() = runTest {
        val service = FakeSettingsService()
        service.failures["sources"] = IllegalStateException("réseau indisponible")
        val model = model(service)
        model.open("sources")

        val card = model.state.sources
        assertNull(card.data)
        assertEquals("réseau indisponible", card.failure)
    }

    /**
     * The theme applies BEFORE the store answers: the switch, the card and the
     * whole app (through [NzAppearance]) move in the same frame, and the write
     * only follows.
     */
    @Test
    fun a_theme_applies_before_the_store_answers() = runTest {
        val service = FakeSettingsService()
        val gate = CompletableDeferred<Unit>()
        service.writeGate = gate
        val model = model(service)
        model.start()
        NzAppearance.load(service)
        assertEquals("encre", model.state.settings?.theme)

        model.setTheme("papier")
        assertEquals("the card already says Papier", "papier", model.state.settings?.theme)
        assertEquals(
            "and the shell paints Papier in the same frame",
            fr.neezer.mobile.designsystem.NzThemeKind.Papier,
            NzAppearance.current.value.theme,
        )
        assertTrue("the write is still in flight", service.writes.isEmpty())

        gate.complete(Unit)
        assertEquals(listOf("ui.theme" to "\"papier\""), service.writes)
    }

    /** A refused write puts the setting back and says why — never a silent change. */
    @Test
    fun a_refused_write_rolls_back_with_its_cause() = runTest {
        val service = FakeSettingsService()
        // A plain throwable: a generated `NzException` subclass would load JNA,
        // and this test runs with no `.so` at all.
        service.failures["set"] = IllegalArgumentException("couleur invalide")
        val gate = CompletableDeferred<Unit>()
        service.writeGate = gate
        val toasts = mutableListOf<String>()
        val model = model(service, toasts)
        model.start()
        NzAppearance.load(service)

        model.setAccent("#7C5CFF")
        assertEquals("the accent applies at once", "#7C5CFF", model.state.settings?.accent)

        gate.complete(Unit)
        assertNull("the accent is back to none", model.state.settings?.accent)
        assertNull("and the shell forgot it too", NzAppearance.current.value.accent)
        assertTrue(toasts.isNotEmpty())
    }

    /** The eco switch is `viz.eco`, and it reaches the shell as « no animation ». */
    @Test
    fun the_eco_switch_reaches_the_shell() = runTest {
        val service = FakeSettingsService()
        val model = model(service)
        model.start()
        model.setEco(true)
        assertEquals(true, model.state.settings?.eco)
        assertEquals(
            fr.neezer.mobile.designsystem.NzMotionLevel.Off,
            NzAppearance.current.value.motion,
        )
        assertEquals(listOf("viz.eco" to "true"), service.writes)
    }

    /** A source's order is drawn at once, and the server's answer replaces it. */
    @Test
    fun an_order_previews_then_takes_the_servers_answer() = runTest {
        val service = FakeSettingsService()
        val model = model(service)
        model.open("sources")

        model.saveSourceOrder(listOf("youtube", "deezer", "qobuz"))
        // The optimistic card: YouTube leads before the request returned.
        assertEquals("youtube", model.state.sources.data?.main)
        assertEquals("main", model.state.sources.data?.sources?.first { it.id == "youtube" }?.place)

        assertTrue("the order travelled", "saveSourcesOrder" in service.calls)
        assertEquals("the server's own answer", "youtube", model.state.sources.data?.main)
    }

    /**
     * One drag of the discovery level is ONE write: the value moves at once, and
     * the save waits for the hand to settle (450 ms, `lib/discovery.js`).
     */
    @Test
    fun one_discovery_drag_saves_once() = runTest {
        val service = FakeSettingsService()
        val model = model(service)
        model.start()

        model.setDiscoveryLevel(0.2)
        model.setDiscoveryLevel(0.3)
        model.setDiscoveryLevel(0.55)
        assertEquals(0.55, model.state.discoveryLevel, 0.0001)
        assertTrue("nothing saved yet", service.writes.isEmpty())

        advanceTimeBy(SettingsModel.SETTLE_MS + 1)
        assertEquals("one write for the whole drag", 1, service.writes.size)
        assertEquals("discover.level" to "0.55", service.writes.single())
    }

    /** The prefetch count is 1…10, as the SPA's `setPrefetchCount` clamps it. */
    @Test
    fun the_prefetch_count_is_clamped() = runTest {
        val service = FakeSettingsService()
        val model = model(service)
        model.start()
        model.setPrefetchCount(0)
        assertEquals(1u, model.state.settings?.prefetchCount)
        model.setPrefetchCount(99)
        assertEquals(10u, model.state.settings?.prefetchCount)
    }

    /**
     * Opening « Assistance » builds the report — the screen DRAWS it, and the
     * share sheet reuses it. Before: only the share button built it, so the
     * counted memory and requests were never on screen (MQ2's journey could not
     * read them).
     */
    @Test
    fun opening_help_builds_the_report() = runTest {
        val service = FakeSettingsService()
        val model = model(service)
        model.open("help")
        val report = model.state.report
        assertNotNull("the report must exist without a tap on « Partager »", report)
        assertTrue(report!!.contains("diagnostic"))
        assertTrue("the report carries the counted memory", report.contains("mémoire comptée"))
        assertEquals("one build at open", 1, service.calls.count { it == "diagnosticReport" })
        // The share button keeps it: no second build, the same text.
        assertEquals(report, model.buildReport())
    }

    /**
     * The build is OFF the opening frame: `open` schedules it, and the report
     * lands when the frame's work is done (rule 2/rule 4 — a screen never waits
     * on work it does not need to paint). A `StandardTestDispatcher` is the
     * point: with an unconfined one the build would run inline and the first
     * assertion would already see the text.
     */
    @Test
    fun opening_help_does_not_block_on_the_report() = runTest {
        val service = FakeSettingsService()
        val dispatcher = StandardTestDispatcher(testScheduler)
        val scope = CoroutineScope(dispatcher)
        scopes += scope
        val model = SettingsModel(service = service, scope = scope, io = dispatcher, toast = {})
        model.open("help")
        assertNull("the open returned before the build ran", model.state.report)
        advanceUntilIdle()
        assertNotNull("the report arrived once the build ran", model.state.report)
    }

    /** A member's 403 is not an error: the administrator's card simply is not drawn. */
    @Test
    fun a_member_sees_no_admin_card() = runTest {
        val service = FakeSettingsService()
        service.failures["serverSettings"] = IllegalStateException("403")
        val model = model(service)
        model.open("sources")
        assertNull(model.state.server)
        assertTrue("the cause is kept for the log, not shown as a card", model.state.serverFailure != null)
    }

    /** A member's *Sources* is the SPA's three requests: no admin card at all. */
    @Test
    fun a_member_asks_three_routes_and_sees_no_admin_card() = runTest {
        val service = FakeSettingsService(server = null, deezer = null, access = SettingsFixtures.access(admin = false))
        val model = model(service)
        model.open("sources")
        assertEquals(listOf("sources", "accounts", "access"), service.calls)
        assertNull(model.state.server)
    }

    /** The categories a person sees, and the filter that narrows them. */
    @Test
    fun the_menu_filters_without_a_route_or_a_request() = runTest {
        val service = FakeSettingsService()
        val model = model(service)
        model.start()
        val all = model.state.categories.flatMap { it.second }
        assertTrue(all.any { it.id == "appearance" })
        assertTrue("no admin page for a member", all.none { it.id == "users" })

        model.setQuery("decouverte")
        val found = model.state.categories.flatMap { it.second }
        assertEquals(listOf("discovery"), found.map { it.id })
    }
}
