package fr.neezer.mobile.designsystem

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.nz_ffi.CoreConfig
import uniffi.nz_ffi.NzCore
import uniffi.nz_ffi.TrackRow

/**
 * `asRowSource()` over a REAL `ListHandle` — the one check the JVM cannot make.
 *
 * `ListWindow`'s adapter used to be `= object : NzRowSource { override fun len()
 * = len() }`, which calls ITSELF: the first screen that windowed a handle died
 * with `StackOverflowError` at `ListWindowKt$asRowSource$1.len(ListWindow.kt:25)`
 * (MB6, emulator, measured). A JVM fake implements `NzRowSource` directly and
 * never goes through the adapter, so `ListWindowTest` was green over a function
 * that could only recurse. This test builds the real object graph, opens 130
 * rows, and reads two windows through the adapter.
 *
 * Runs on the emulator `mobile/tools/emu.sh` starts
 * (`:designsystem:connectedDebugAndroidTest`, discovered by
 * `mobile/tools/ci-emulator.sh`).
 */
@RunWith(AndroidJUnit4::class)
class RowSourceTest {

    private fun rows(count: Int): List<TrackRow> = (0 until count).map { number ->
        TrackRow(
            key = "row:$number",
            id = "id-$number",
            title = "Titre $number",
            artists = "Artiste",
            album = "Album",
            albumId = null,
            durationS = 180u,
            cover = null,
            explicit = false,
            liked = false,
            disliked = false,
            unavailable = false,
            downloaded = false,
            source = "deezer",
            position = number.toUInt(),
        )
    }

    @Test
    fun aRealHandleDelegatesItsLengthAndItsWindows() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val root = context.cacheDir.resolve("row-source-test")
        val core = NzCore(
            CoreConfig(
                dataDir = root.resolve("data").apply { mkdirs() }.absolutePath,
                cacheDir = root.resolve("cache").apply { mkdirs() }.absolutePath,
                appVersion = "0.0.0-test",
                versionCode = 2_000_001u,
                sdkInt = 34u,
                deviceName = "instrumented",
            ),
        )
        val handle = core.openTrackList(rows(130))
        val source = handle.asRowSource()

        assertEquals("130 rows cross the adapter", 130, source.len())
        assertEquals("a page is what was asked for", 10, source.window(0, 10).size)
        assertEquals("Titre 5", source.window(5, 1).single().title)
        assertEquals(
            "the window starts where it was told",
            "Titre 120",
            source.window(120, 10).first().title,
        )
        assertEquals(
            "a window that runs past the end is clipped to the rows there are",
            5,
            source.window(125, 10).size,
        )
        // The window policy over the same source: one page under the viewport.
        val window = ListWindow(source)
        window.onVisible(0, 9, scrolling = false)
        assertEquals("one page for the first screen", 1, window.requestsForFling)
        assertEquals("Titre 0", window.rowAt(0)?.title)
        core.shutdown()
    }
}
