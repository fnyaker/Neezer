package fr.neezer.mobile.baselineprofile

import android.os.SystemClock
import androidx.benchmark.macro.MacrobenchmarkScope
import androidx.test.uiautomator.By
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.UiObject2
import androidx.test.uiautomator.Until
import org.junit.Assume

/**
 * The one scripted journey of the phone — the desktop's D17 for `mobile/`
 * (docs/agents/mobile.md, MQ2).
 *
 * A person on a bad mobile link opens the app, searches, opens an album, plays
 * it, walks the two worst lists (8 000 favourites, a 4 000-track playlist),
 * opens the player and its queue, reads the diagnostic, and — in
 * [JourneyLongTest]'s half — downloads a 50-track playlist, pulls the plug
 * (airplane mode), plays the downloads and lets the phone run forty titles
 * behind a locked screen with Doze forced.
 *
 * **Reached by a path, never by a coordinate, and proved by a node.** Every step
 * waits on what the screen itself draws (a `contentDescription` marker, or the
 * French label a person reads) and fails naming what it never saw: a journey
 * that skips a screen measures the previous one, and D17's first audit found a
 * whole screen nothing could reach. Compose puts a tap's ACTION on a merged
 * parent while the Text carrying the label reports `clickable=false`, so a step
 * taps a node's own bounds ([tapText], [tapFirstCardBelow]) instead of asking
 * the accessibility tree for a clickable label — that request finds nothing, and
 * a bench that "found nothing" once measured the screen before (measured on this
 * emulator, 2026-11-23).
 *
 * **What the numbers are.** The app's own (`Assistance`'s diagnostic report:
 * per-tag request/byte counters are `NzCore::stats()`, `mémoire comptée` is
 * `NzCore::memory()`), the device's (`/proc/<pid>/stat`, `dumpsys meminfo`,
 * `dumpsys media_session`) or the engine's, read in process by the lot that owns
 * it — never a shared test process's RSS and never a timer invented here.
 */
object Journey {

    /** The account `tools/perf_api.py --serve` seeds, as the bench knows it. */
    const val PACKAGE = BenchTarget.PACKAGE_NAME

    /** Markers the screens draw in every state (the note's own contract). */
    const val HOME = "Accueil Neezer"
    const val SEARCH = "Recherche Neezer"
    const val ALBUM = "Fiche album"
    const val LIBRARY = "Bibliothèque Neezer"

    /** The two leaves of the library this journey walks. */
    const val LEAF_TITLES = "Titres"
    const val LEAF_PLAYLISTS = "Playlists"

    /** The seeded playlists, by the names `perf_api.py`'s fixed seed gives them. */
    const val DOWNLOAD_PLAYLIST = "Playlist 37"
    const val BIG_PLAYLIST = "Playlist 0"

    /** The query both the SPA's bench and this journey search: broad, ASCII. */
    const val PLAY_ALBUM_QUERY = "storm"

    /** How long a screen gets to paint before the journey calls it missing. */
    const val SCREEN_TIMEOUT_MS = 20_000L

    /** The 50 tiny archived titles a downloaded playlist fetches over the LAN. */
    const val DOWNLOAD_TIMEOUT_MS = 180_000L

    // -- waits, on events ---------------------------------------------------

    /** Waits for a node with exactly [text]; fails naming it, never silently. */
    fun awaitText(device: UiDevice, text: String, timeoutMs: Long = SCREEN_TIMEOUT_MS): UiObject2 {
        val found = device.wait(Until.findObject(By.text(text)), timeoutMs)
        check(found != null) { "the screen never showed « $text » within ${timeoutMs}ms" }
        return found
    }

    /** Waits for a node whose text CONTAINS [needle] (a title inside a row). */
    fun awaitContains(device: UiDevice, needle: String, timeoutMs: Long = SCREEN_TIMEOUT_MS): UiObject2 {
        val found = device.wait(Until.findObject(By.textContains(needle)), timeoutMs)
        check(found != null) { "no node containing « $needle » within ${timeoutMs}ms" }
        return found
    }

    /** Waits for a node whose content description is exactly [desc]. */
    fun awaitDesc(device: UiDevice, desc: String, timeoutMs: Long = SCREEN_TIMEOUT_MS): UiObject2 {
        val found = device.wait(Until.findObject(By.desc(desc)), timeoutMs)
        check(found != null) { "no node with the description « $desc » within ${timeoutMs}ms" }
        return found
    }

    /**
     * Taps the node whose text is exactly [text], by the node's own bounds.
     *
     * `UiObject2.click()` taps the centre of the node it holds; Compose's tap
     * action lives on a merged parent whose child Text reports
     * `clickable=false`, so a search for `isClickable` finds nothing while this
     * lands on the control.
     *
     * The handle is RE-FOUND when it goes stale: a screen that recomposes between
     * the find and the click (a search's results landing, a chip row arriving)
     * kills the run with `StaleObjectException` — measured on this lot's dispatch
     * (run 45778420506) in `searchToAlbum`, which killed `playerOpen` before it
     * measured a frame. Three attempts, each a fresh find: the tap is the event,
     * the handle is never the contract.
     */
    fun tapText(device: UiDevice, text: String, timeoutMs: Long = SCREEN_TIMEOUT_MS) {
        tapAndWait(device, timeoutMs) { awaitText(device, text, timeoutMs) }
    }

    /** Taps a node by its content description, the same way. */
    fun tapDesc(device: UiDevice, desc: String, timeoutMs: Long = SCREEN_TIMEOUT_MS) {
        tapAndWait(device, timeoutMs) { awaitDesc(device, desc, timeoutMs) }
    }

    private fun tapAndWait(device: UiDevice, timeoutMs: Long, find: () -> UiObject2) {
        var last: Throwable? = null
        repeat(3) {
            // `find` throws when the node is simply not there yet (its own message
            // names the label); a stale handle is the retryable one.
            val node = find()
            val clicked = runCatching { node.click() }
            if (clicked.isSuccess) {
                device.waitForIdle()
                return
            }
            last = clicked.exceptionOrNull()
            device.waitForIdle()
        }
        throw checkNotNull(last) { "the tap on the node never happened ($timeoutMs ms)" }
    }

    /**
     * The first CLICKABLE node below the LAST node whose text is [header] — the
     * card of a grid (its title and cover are children of a text-less clickable
     * column; tapping the title does nothing, measured).
     */
    fun tapFirstCardBelow(device: UiDevice, header: String, timeoutMs: Long = SCREEN_TIMEOUT_MS) {
        val deadline = SystemClock.uptimeMillis() + timeoutMs
        var sawHeader = false
        while (SystemClock.uptimeMillis() < deadline) {
            // Both reads go STALE when the screen recomposes between the find and
            // the bounds read (measured: a search result's heading went stale on
            // a loaded machine and killed the whole benchmark with
            // `StaleObjectException`). Re-find and re-read instead of trusting a
            // handle held across a recomposition.
            val headings = runCatching {
                device.findObjects(By.text(header)).mapNotNull { node ->
                    runCatching { node.visibleBounds }.getOrNull()
                }
            }.getOrDefault(emptyList())
            if (headings.isNotEmpty()) sawHeader = true
            val below = headings.maxByOrNull { it.top }?.bottom
            if (below != null) {
                val card = runCatching {
                    device.findObjects(By.clickable(true))
                        .mapNotNull { node ->
                            runCatching { node.visibleBounds }.getOrNull()?.let { node to it }
                        }
                        .filter { (_, bounds) -> bounds.top >= below && bounds.height() > 100 }
                        .minByOrNull { (_, bounds) -> bounds.top }
                }.getOrNull()
                if (card != null) {
                    runCatching { card.first.click() }
                        .onSuccess {
                            device.waitForIdle()
                            return
                        }
                }
            }
            device.waitForIdle()
        }
        check(sawHeader) { "the screen never showed « $header »" }
        error("no card under « $header » within ${timeoutMs}ms")
    }

    /**
     * Waits for [text], scrolling the screen DOWN when it is not there yet: the
     * Réglages list is longer than a phone, and « Assistance » sits under
     * « Comptes » (measured: no amount of waiting brings it on screen).
     */
    fun scrollToText(device: UiDevice, text: String, timeoutMs: Long = SCREEN_TIMEOUT_MS): UiObject2 {
        // From the TOP first: a screen left scrolled at its bottom (Réglages after
        // Assistance) would otherwise only ever scroll further away from a row
        // that lives above (measured).
        repeat(4) { fling(device, up = false) }
        var scrolls = 0
        while (scrolls <= 8) {
            val found = device.findObject(By.text(text))
            if (found != null) return found
            fling(device, up = true)
            scrolls += 1
        }
        error("« $text » never came on screen after $scrolls scrolls")
    }

    /**
     * The first node whose text starts with [title] that is NOT a text field,
     * scrolling DOWN while it is not there (the 60 seeded playlists are one
     * column of cards; only the first few fit).
     */
    fun awaitCard(device: UiDevice, title: String, timeoutMs: Long = SCREEN_TIMEOUT_MS): UiObject2 {
        val deadline = SystemClock.uptimeMillis() + timeoutMs
        while (SystemClock.uptimeMillis() < deadline) {
            val card = device.findObjects(By.textStartsWith(title))
                .firstOrNull { it.className != "android.widget.EditText" }
            if (card != null) return card
            fling(device, up = true)
        }
        error("no card titled « $title » within ${timeoutMs}ms")
    }

    /**
     * One vertical fling over the content area: a third of the width off-centre,
     * so a row's swipe action (MB10's right-swipe) is not what is measured.
     */
    fun fling(device: UiDevice, up: Boolean = true) {
        val w = device.displayWidth
        val h = device.displayHeight
        val x = (w * 0.6f).toInt()
        val from = if (up) (h * 0.75f).toInt() else (h * 0.30f).toInt()
        val to = if (up) (h * 0.30f).toInt() else (h * 0.75f).toInt()
        device.swipe(x, from, x, to, 24)
        device.waitForIdle()
    }

    /** The first editable field on screen (a Compose text field is an EditText). */
    fun firstField(device: UiDevice, timeoutMs: Long = SCREEN_TIMEOUT_MS): UiObject2 {
        val field = device.wait(Until.findObject(By.clazz("android.widget.EditText")), timeoutMs)
        check(field != null) { "no text field on this screen within ${timeoutMs}ms" }
        return field
    }

    /**
     * Types [text] into the screen's one field: a tap, then the device's own key
     * stream. ASCII on purpose — `input text` cannot carry an accent, and a bench
     * that types one measures a silently empty query.
     *
     * Three attempts, and the field's own text is the proof: a Compose field that
     * has not taken the focus yet drops the keys while `input text` reports
     * success (measured: the field stayed empty and the run measured the empty
     * screen).
     */
    fun type(device: UiDevice, text: String) {
        var typed = ""
        repeat(3) {
            // A node goes STALE when the screen behind it recomposes between the
            // find and the tap (measured on the playlists' filter): re-find it and
            // tap again instead of failing a journey that is working.
            val field = firstField(device)
            // BOTH clicks are guarded: the fallback's own handle goes stale the
            // same way the first one does, and an unguarded `.click()` there is
            // exactly the `StaleObjectException` that killed `playerOpen` before
            // its first frame (run 45782321113).
            runCatching { field.click() }
                .onFailure { runCatching { firstField(device).click() } }
            device.waitForIdle()
            // A Compose field drops injected keys until it owns the focus; the
            // IME connection can be a few frames behind the tap.
            val focused = SystemClock.uptimeMillis() + 1_500
            while (SystemClock.uptimeMillis() < focused) {
                if (runCatching { firstField(device).isFocused }.getOrDefault(false)) break
                device.waitForIdle()
            }
            if (!(field.text ?: "").isNullOrBlank()) {
                device.executeShellCommand("input keyevent KEYCODE_MOVE_END")
                repeat(48) { device.executeShellCommand("input keyevent KEYCODE_DEL") }
                device.waitForIdle()
            }
            device.executeShellCommand("input text ${text.replace(" ", "%s")}")
            // The field's own text can lag the injected keys by a frame or two:
            // poll it, then judge (a single read called a working query empty).
            val until = SystemClock.uptimeMillis() + 3_000
            while (SystemClock.uptimeMillis() < until) {
                // A read on a node the screen recomposed under throws; an empty
                // read is what the loop already handles.
                typed = runCatching { firstField(device).text ?: "" }.getOrDefault("")
                if (typed.contains(text, ignoreCase = true)) return
                device.waitForIdle()
            }
            // The semantics action is the second door: some builds take it where
            // the key stream is dropped.
            runCatching { firstField(device).setText(text) }
            device.waitForIdle()
            typed = runCatching { firstField(device).text ?: "" }.getOrDefault("")
            if (typed.contains(text, ignoreCase = true)) return
        }
        error("the field holds « $typed » and not « $text »: the query never reached the app")
    }

    /**
     * Hides the keyboard when — and only when — it is shown.
     *
     * `mobile/tools/ui.sh`'s `hide_keyboard`, for the same measured reason: a
     * Compose field can take its text through the semantics action without ever
     * opening the IME, and a BACK pressed over a closed keyboard LEAVES the
     * screen — the next wait then names a card that is on the previous page
     * (measured on `playlistScroll4000`, run 45782321113: « no card titled
     * « Playlist 0 » »).
     */
    fun hideKeyboard(device: UiDevice) {
        val shown = runCatching {
            device.executeShellCommand("dumpsys input_method").contains("mInputShown=true")
        }.getOrDefault(false)
        if (shown) {
            device.pressBack()
            device.waitForIdle()
        }
    }

    // -- the steps ----------------------------------------------------------

    /** Launches to the home: the shell's first tab is the event, not a sleep. */
    fun launchToHome(scope: MacrobenchmarkScope) {
        scope.pressHome()
        scope.startActivityAndWait()
        awaitDesc(scope.device, HOME)
    }

    /**
     * The account the journey stands on: the home greets the person by NAME only
     * when `restore()`/`/api/me` answered (`BONJOUR, BENCH · LUNDI`). Without it
     * the app is a fresh install with no server, and every step below measures an
     * empty screen — so the run is SKIPPED with its cause, which is what keeps
     * the CI emulator job green (it has no bench server) without pretending.
     */
    fun requireBenchAccount(device: UiDevice) {
        val named = device.wait(Until.findObject(By.textContains("BONJOUR,")), 10_000) != null
        Assume.assumeTrue(
            "the app has no signed-in bench account: MQ2's journey measures an app configured " +
                "against mobile/tools/perf-server.sh (8 000 titles, 8 000 favourites)",
            named,
        )
    }

    /**
     * A cold start from a plain test: Macrobenchmark kills the app before each
     * iteration, a JUnit test does not — and resuming a task left on another
     * screen is not the journey's first step (measured: the run resumed on the
     * search screen and waited for « Accueil » forever).
     */
    fun restart(scope: MacrobenchmarkScope) {
        scope.device.executeShellCommand("am force-stop ${BenchTarget.PACKAGE_NAME}")
        launchToHome(scope)
        requireBenchAccount(scope.device)
    }

    /**
     * The journey's fixture, stated as a precondition instead of a failure: the
     * measure needs `perf-server.sh` (8 000 titles, 8 000 favourites, the
     * 4 000-track playlist) and an app signed in to it. A CI emulator has
     * neither, and a benchmark that cannot run says WHY (`ScrollBenchmark`'s own
     * rule) instead of timing an empty screen — the fixture's marker is the
     * library header the seeded server draws.
     */
    fun requireSeededLibrary(device: UiDevice) {
        val seeded = device.wait(Until.findObject(By.textContains("8000 titres")), 10_000) != null
        Assume.assumeTrue(
            "the bench library is not on this app: start mobile/tools/perf-server.sh " +
                "--favorites 8000 and sign the app in to it (MQ2's journey measures that server)",
            seeded,
        )
    }

    /** Home → search « storm » → the albums tab → the first card's screen. */
    fun searchToAlbum(scope: MacrobenchmarkScope) {
        val device = scope.device
        tapText(device, BenchTarget.TAB_SEARCH)
        awaitDesc(device, SEARCH)
        type(device, PLAY_ALBUM_QUERY)
        // The results' sections are headed « Titres », « Artistes », « Albums »…
        // The chip of the same name narrows the list to the albums; the heading
        // under it is what the card is found below.
        val albums = device.wait(Until.findObject(By.text("Albums")), 10_000) != null
        Assume.assumeTrue(
            "the search answered nothing: the app has no bench library (MQ2 needs perf-server)",
            albums,
        )
        tapText(device, "Albums")
        tapFirstCardBelow(device, "Albums")
        awaitDesc(device, ALBUM)
    }

    /**
     * The album's « Lire », and the UI's own answer to the tap: the mini-player's
     * button says « Pause » once the app is playing.
     *
     * Returns the milliseconds from the tap to that node — an UPPER BOUND of the
     * engine's `output_stats()` figure (the same transition, plus one
     * accessibility round trip). The plan's ceilings apply to the engine number
     * (`output_stats().last_first_audible_ns - last_tap_ns`, measured in process
     * by MB7/MB11's device tests); this one is reported beside it as the number a
     * person's eye can check.
     */
    fun playAlbum(scope: MacrobenchmarkScope): Long {
        val device = scope.device
        val tapAt = SystemClock.uptimeMillis()
        tapText(device, "Lire")
        awaitDesc(device, "Pause", 30_000)
        return SystemClock.uptimeMillis() - tapAt
    }

    /**
     * The Bibliothèque tab, from wherever the shell currently is.
     *
     * The tab RESTORES its own back stack (`NavGraph.open`'s `restoreState`): a
     * person who left the library on a playlist comes back to that playlist, so
     * waiting for the library's marker after the tap waits forever (measured).
     * Walk out with the system's back until the library is the screen — exactly
     * what the person does.
     */
    fun ensureLibrary(device: UiDevice) {
        tapText(device, BenchTarget.TAB_LIBRARY)
        repeat(4) {
            // A bounded wait, not an instantaneous read: the destination paints a
            // frame or two after the tap, and a `findObject` that missed it sent
            // the next back from the LIBRARY to the home (measured).
            if (device.wait(Until.hasObject(By.desc(LIBRARY)), 3_000)) {
                requireSeededLibrary(device)
                return
            }
            device.pressBack()
            device.waitForIdle()
        }
        error("the library never came back after four backs")
    }

    /** Library → Titres (the 8 000 favourites) → four flings down and back. */
    fun flingFavorites(scope: MacrobenchmarkScope) {
        val device = scope.device
        ensureLibrary(device)
        tapText(device, LEAF_TITLES)
        // The favourites' own tools (« Tout lire »/« Mélanger ») are drawn only
        // once the 8 000 rows are in: their presence is the list's event.
        awaitText(device, "Mélanger", 30_000)
        check(device.findObject(By.scrollable(true)) != null) {
            "the favourites leaf painted no scrollable list"
        }
        repeat(4) { fling(device, up = true) }
        repeat(4) { fling(device, up = false) }
    }

    /**
     * Library → Playlists → the 4 000-row playlist → four flings down and back,
     * then « Lire »: the queue this journey edits and soaks is the 4 000-row one,
     * not the two-title album (whose 4.8 s titles ended before the player step,
     * measured).
     */
    fun flingPlaylist(scope: MacrobenchmarkScope) {
        val device = scope.device
        openPlaylist(device, BIG_PLAYLIST)
        repeat(4) { fling(device, up = true) }
        repeat(4) { fling(device, up = false) }
        val tapAt = SystemClock.uptimeMillis()
        tapText(device, "Lire")
        // The bar's « Pause » is the state's own event; a playlist whose first
        // title the server is still transcoding may take a moment, so the wait is
        // bounded and its outcome PRINTED — the capture tool screenshots a
        // journey, it does not own the playback acceptance (the journey
        // benchmark's diagnostic report and MB3c's tests do).
        val playing = runCatching { awaitDesc(device, "Pause", 20_000) }.isSuccess
        println(
            "MEASURE journey playlist Lire → « Pause » : " +
                "${SystemClock.uptimeMillis() - tapAt}ms (barre=${if (playing) "oui" else "non"})",
        )
    }

    /** Library → Playlists → the named playlist, open on its own screen. */
    fun openPlaylist(device: UiDevice, name: String) {
        ensureLibrary(device)
        tapText(device, LEAF_PLAYLISTS)
        // The playlists arrive under « Sans dossier », and the section starts
        // FOLDED: a screenshot showed the chevron and no card while the tree
        // looked like an empty list. The CHEVRON is the toggle (the label's own
        // centre does nothing — measured); the count beside the label is the
        // account's total, not the filter's, so the filter is no proof either.
        type(device, name)
        hideKeyboard(device)
        // The query narrows the section to the one card — but ONLY once the
        // section is unfolded, and a fold has no text to read: the chevron's
        // state is not in the tree. So the card is looked for first, and the
        // heading toggled ONCE when it is not there. (The old order toggled
        // before filtering, on "no card yet", which could also CLOSE a section
        // that was already open — measured on run 45782321113.)
        var found: UiObject2? = runCatching { awaitCard(device, name, 12_000) }.getOrNull()
        if (found == null) {
            // The CHEVRON is the toggle (the label's own centre does nothing —
            // measured); the count beside the label is the account's total.
            // `scrollToText`, not `awaitText`: the first look for the card flung
            // the list, so the heading may be above the fold.
            val head = scrollToText(device, "Sans dossier")
            val b = runCatching { head.visibleBounds }.getOrNull()
            if (b != null) {
                device.click((b.left - 60).coerceAtLeast(8), b.centerY())
                device.waitForIdle()
            }
            found = awaitCard(device, name)
        }
        // NOT `awaitContains`: the filter field holds the very query, and its
        // node comes first in the tree — clicking it taps nothing (measured: the
        // run then waited for « Lire » on a screen that never changed). The tap
        // is retried on a fresh handle: the list recomposes while the filter's
        // results land.
        val card = checkNotNull(found) { "no card titled « $name »" }
        runCatching { card.click() }
            .onFailure { runCatching { awaitCard(device, name).click() } }
        device.waitForIdle()
        awaitText(device, "Lire", 30_000)
    }

    /** Playlists → the 50-track playlist, downloaded whole (MB15). */
    fun downloadPlaylist(scope: MacrobenchmarkScope) {
        val device = scope.device
        openPlaylist(device, DOWNLOAD_PLAYLIST)
        tapText(device, "Télécharger")
        // « Téléchargé » is the control's own done state, not a countdown.
        awaitText(device, "Téléchargé", DOWNLOAD_TIMEOUT_MS)
    }

    /**
     * The full-screen player, through the door the home draws — « Reprendre ».
     *
     * NOT the mini bar before this lot's fix: the bar's model read the player's
     * snapshot once and never followed `PlayerFeed`, so no bar existed over a
     * running player (measured 2026-11-23; the fix and its test are PR #380).
     * Now the bar is there, and this journey still takes the home's card because
     * it is the door a cold reader finds first.
     */
    fun openPlayer(scope: MacrobenchmarkScope) {
        val device = scope.device
        tapText(device, BenchTarget.TAB_HOME)
        awaitDesc(device, HOME)
        awaitText(device, "REPRENDRE", 30_000)
        tapFirstCardBelow(device, "REPRENDRE")
        awaitDesc(device, "Réduire le lecteur")
    }

    /** The player's queue pane, and its own drag: one row up, never the playing one. */
    fun openQueue(scope: MacrobenchmarkScope) {
        val device = scope.device
        tapText(device, "File ·")
        awaitText(device, "File d'attente")
        val handle = device.wait(
            Until.findObject(By.descStartsWith("Déplacer « ")),
            SCREEN_TIMEOUT_MS,
        )
        if (handle != null) {
            val b = handle.visibleBounds
            device.swipe(b.centerX(), b.centerY(), b.centerX(), b.centerY() - b.height() * 2, 24)
            device.waitForIdle()
        }
    }

    /** Back to the shell: the queue pane, then the sheet. */
    fun closePlayer(scope: MacrobenchmarkScope) {
        val device = scope.device
        runCatching { device.findObject(By.desc("Fermer la file d'attente"))?.click() }
        device.waitForIdle()
        tapDesc(device, "Réduire le lecteur")
    }

    /** The whole player step, as the benchmark walks it. */
    fun openPlayerAndQueue(scope: MacrobenchmarkScope) {
        openPlayer(scope)
        openQueue(scope)
        closePlayer(scope)
    }

    /**
     * Réglages → Assistance, and the diagnostic report as the screen draws it:
     * the app's own counted bytes and per-tag request/byte counters.
     *
     * The caller is left on Réglages (one back from the shell it came from).
     */
    fun readDiagnostic(scope: MacrobenchmarkScope): Diagnostic {
        val device = scope.device
        tapText(device, BenchTarget.ACTION_SETTINGS)
        scrollToText(device, "Assistance").click()
        device.waitForIdle()
        awaitText(device, "Partager le rapport", 30_000)
        val node = device.wait(Until.findObject(By.textContains("requêtes par écran")), 30_000)
        check(node != null) { "the diagnostic report never painted" }
        val report = node.text ?: ""
        device.pressBack()
        device.waitForIdle()
        return Diagnostic.parse(report)
    }

    /** The visualiser, through Réglages → Animations → its own door. */
    fun openVisualizer(scope: MacrobenchmarkScope) {
        val device = scope.device
        tapText(device, BenchTarget.ACTION_SETTINGS)
        scrollToText(device, "Animations").click()
        device.waitForIdle()
        awaitText(device, "Ouvrir le visualiseur", 30_000)
        tapText(device, "Ouvrir le visualiseur")
        // The scene's own controls: the sheet is behind, the scene draws its
        // palette and intensity pickers.
        awaitText(device, "Palette", 30_000)
    }

    /**
     * The link down (`airplane_mode_on`), the way a person does it; the app's
     * `ConnectivityManager` callback flips it offline in zero sockets.
     */
    fun airplaneMode(device: UiDevice, on: Boolean) {
        device.executeShellCommand(
            "cmd connectivity airplane-mode ${if (on) "enable" else "disable"}",
        )
        val want = if (on) "1" else "0"
        val deadline = SystemClock.uptimeMillis() + SCREEN_TIMEOUT_MS
        while (SystemClock.uptimeMillis() < deadline) {
            val got = device.executeShellCommand("settings get global airplane_mode_on").trim()
            if (got == want) return
            device.waitForIdle()
        }
        error("airplane mode never became $want")
    }

    /**
     * Writes one `name=value` line to /data/local/tmp/mq2/measures.txt ON THE
     * DEVICE: `am instrument` does not forward a test's stdout, and a number that
     * only exists in a console nobody can read is not a measurement. Base64, so
     * no quoting rule of a shell can eat the value.
     */
    fun record(device: UiDevice, name: String, value: String) {
        // `am instrument` does not print a test's stdout on the console, but
        // Android puts it in logcat under `System.out` — the run's numbers are
        // read from there (`adb logcat -s System.out | grep MEASURE`).
        println("MEASURE $name=$value")
        val line = "$name=$value\n"
        val encoded = android.util.Base64.encodeToString(line.toByteArray(), android.util.Base64.NO_WRAP)
        device.executeShellCommand("mkdir -p /data/local/tmp/mq2")
        device.executeShellCommand("sh -c 'echo $encoded | base64 -d >> /data/local/tmp/mq2/measures.txt'")
    }

    /** The report's counted bytes and requests, one line per tag. */
    fun record(device: UiDevice, diagnostic: Diagnostic, label: String) {
        record(device, "$label.memoire_comptee_o", diagnostic.countedBytes.toString())
        record(device, "$label.file_o", diagnostic.queueBytes.toString())
        record(device, "$label.listes_o", diagnostic.listBytes.toString())
        record(device, "$label.rebuffers", diagnostic.rebuffers.toString())
        for ((tag, stats) in diagnostic.tags) {
            record(device, "$label.$tag.requetes", stats.requests.toString())
            record(device, "$label.$tag.octets_recus", stats.bytesIn.toString())
            record(device, "$label.$tag.octets_envoyes", stats.bytesOut.toString())
        }
    }

    /**
     * The app's diagnostic report, parsed: `Assistance` shows `NzCore::stats()`
     * and `NzCore::memory()` — counted bytes and counted requests, never a guess.
     */
    data class Diagnostic(
        val queueBytes: Long,
        val listBytes: Long,
        /** The player's counted rebuffers (`Core::rebuffers`, never a guess). */
        val rebuffers: Long,
        val online: Boolean,
        val tags: Map<String, Tag>,
    ) {
        data class Tag(val requests: Long, val bytesIn: Long, val bytesOut: Long)

        val countedBytes: Long get() = queueBytes + listBytes

        companion object {
            /**
             * `mémoire comptée : file N o, listes M o`, `rebuffers : N` + one
             * `tag : R requêtes, …`.
             */
            fun parse(text: String): Diagnostic {
                val memory = Regex("mémoire comptée : file (\\d+) o, listes (\\d+) o")
                    .find(text)
                    ?: error("the report has no counted memory:\n$text")
                val rebuffers = Regex("(?m)^rebuffers : (\\d+)$")
                    .find(text)
                    ?.groupValues
                    ?.get(1)
                    ?.toLong()
                    ?: error("the report has no rebuffer count:\n$text")
                val online = !text.contains("réseau : \"hors ligne\"")
                val tags = linkedMapOf<String, Tag>()
                Regex(
                    "^\\s+(\\S+) : (\\d+) requêtes, (\\d+) o reçus, (\\d+) o envoyés",
                    RegexOption.MULTILINE,
                ).findAll(text).forEach { m ->
                    tags[m.groupValues[1]] = Tag(
                        requests = m.groupValues[2].toLong(),
                        bytesIn = m.groupValues[3].toLong(),
                        bytesOut = m.groupValues[4].toLong(),
                    )
                }
                return Diagnostic(
                    queueBytes = memory.groupValues[1].toLong(),
                    listBytes = memory.groupValues[2].toLong(),
                    rebuffers = rebuffers,
                    online = online,
                    tags = tags,
                )
            }
        }
    }
}
