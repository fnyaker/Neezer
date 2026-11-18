package fr.neezer.mobile.feature.offline

import android.graphics.Bitmap
import androidx.activity.ComponentActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.unit.Density
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import fr.neezer.mobile.designsystem.NzMotionLevel
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.NzTheme
import fr.neezer.mobile.designsystem.NzThemeKind
import fr.neezer.mobile.designsystem.Sillon
import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import org.junit.After
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.nz_ffi.DownloadEntry
import uniffi.nz_ffi.ListHandle
import uniffi.nz_ffi.OfflineListener
import uniffi.nz_ffi.OfflineSettings
import uniffi.nz_ffi.OfflineState
import uniffi.nz_ffi.SetProgress
import uniffi.nz_ffi.StorageReport

/**
 * The acceptance's **real-screen capture**: Réglages → Téléchargements and the
 * download button's states, rendered by the REAL Compose runtime on the headless
 * emulator, written into the test app's own files, from where the lot pulls
 * them (`adb -s emulator-<port> pull`).
 *
 * It is an instrumented test, not a debug activity in `:app`: a library module
 * may not open a door into the app's manifest for a screenshot, and the test APK
 * already has the one activity Compose needs. Nothing is drawn on the person's
 * display (`mobile/tools/emu.sh start` is `-no-window`), and the bitmap is read
 * from the decor view the test itself built.
 *
 * The state it paints is a fake port's (the device has no session), so what this
 * proves is what the JVM render cannot: the real fonts, the real device density,
 * TalkBack's tree and the 200 % scale.
 */
@RunWith(AndroidJUnit4::class)
class OfflineCaptureTest {
    @get:Rule
    val rule = createAndroidComposeRule<ComponentActivity>()

    @After
    fun forget() {
        OfflineHost.reset()
    }

    private fun install(): DownloadsModel {
        val model = DownloadsModel(
            port = DevicePort(),
            scope = CoroutineScope(Dispatchers.Main.immediate),
            io = Dispatchers.Main.immediate,
            freeSpace = { 8L * 1024 * 1024 * 1024 },
            toast = { _, _, _ -> },
            causeText = { it.message ?: "erreur" },
        )
        OfflineHost.install(model)
        model.start()
        return model
    }

    private fun capture(
        name: String,
        theme: NzThemeKind,
        fontScale: Float = 1f,
        content: @Composable () -> Unit,
    ) {
        val model = install()
        // The window's own background is the theme's ground: the test activity's
        // default light grey showed through under the compiled section and read
        // as a band across the capture (measured: the region below the content
        // was `#EDEDED` under a white title).
        val windowGround = Sillon.theme(theme).bg.toArgb()
        rule.activity.runOnUiThread {
            rule.activity.window.setBackgroundDrawable(
                android.graphics.drawable.ColorDrawable(windowGround),
            )
        }
        rule.setContent {
            val density = LocalDensity.current
            CompositionLocalProvider(LocalDensity provides Density(density.density, fontScale)) {
                NzTheme(kind = theme, motion = NzMotionLevel.Full) {
                    Box(Modifier.fillMaxSize().background(Sillon.colors.bg).clipToBounds().testTag("capture")) {
                        Column(
                            modifier = Modifier
                                .fillMaxWidth()
                                .verticalScroll(rememberScrollState())
                                .padding(horizontal = NzSpacing.pageX, vertical = NzSpacing.l),
                            verticalArrangement = Arrangement.spacedBy(NzSpacing.l),
                        ) {
                            content()
                        }
                    }
                }
            }
        }
        rule.waitForIdle()
        // The model must have been read by now: a screen painted from nothing
        // would be a screenshot of an empty page.
        check(model.state.value.version > 0uL) { "the section painted nothing" }
        // The COMPOSED node, not the window: `captureToImage` reads the node's
        // own bounds and clips to them, so the activity's window background can
        // never appear as a grey band under the section (measured: the decor
        // view's background showed through where the content ended).
        val full = rule.onNodeWithTag("capture").captureToImage().asAndroidBitmap()
        // The decor is the activity's window: below the section's ground the
        // window's own background shows as a band. Crop it away — the image is
        // the SCREEN, not the window handle (a row is kept while anything in it
        // differs from the band's own colour).
        val ground = Sillon.theme(theme).bg.toArgb()
        var bottom = full.height - 1
        while (bottom > 0) {
            var empty = true
            for (x in 0 until full.width step 4) {
                if (full.getPixel(x, bottom) != ground) {
                    empty = false
                    break
                }
            }
            if (!empty) break
            bottom -= 1
        }
        val bitmap = Bitmap.createBitmap(full, 0, 0, full.width, (bottom + 1).coerceAtLeast(1))
        val dir = File(
            InstrumentationRegistry.getInstrumentation().targetContext.filesDir,
            "mb15-screenshots",
        ).apply { mkdirs() }
        val file = File(dir, name)
        file.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        println("SCREENSHOT $name ${bitmap.width}x${bitmap.height} ${file.absolutePath} ${file.length()} bytes")
    }

    @Test
    fun the_downloads_section_renders_on_the_device() {
        capture("downloads-encre.png", NzThemeKind.Encre) { OfflineSettingsSection() }
        rule.onNodeWithTag("capture").assertExists()
    }

    @Test
    fun the_downloads_section_renders_in_papier_at_200_percent() {
        capture("downloads-papier-200.png", NzThemeKind.Papier, fontScale = 2f) {
            OfflineSettingsSection()
        }
        rule.onNodeWithTag("capture").assertExists()
    }

    @Test
    fun the_download_button_states_render_on_the_device() {
        capture("boutons.png", NzThemeKind.Encre) {
            Column(verticalArrangement = Arrangement.spacedBy(NzSpacing.m)) {
                // Absent, running, done and failed: the fake carries the three
                // contexts below, and `playlist:1` is a context nobody queued.
                DownloadButton(context = "playlist:1")
                DownloadButton(context = "playlist:42")
                DownloadButton(context = "album:404")
                DownloadButton(context = "show:9")
            }
        }
        rule.onNodeWithTag("capture").assertExists()
    }

    /**
     * The device's port: the state the JVM bench uses, minus the calls a test
     * cannot make on a phone without a session.
     */
    private class DevicePort : OfflinePort {
        private var listener: OfflineListener? = null
        private val state = deviceState()
        private val entries = List(200) { index ->
            DownloadEntry(
                key = "31355$index",
                title = "Titre $index",
                artist = "Artiste",
                cover = null,
                durationS = 200u,
                explicit = false,
                quality = if (index % 3 == 0) "FLAC" else "OPUS_320",
                bytes = (4_000_000 + index * 37_000).toULong(),
                addedAt = index.toLong(),
            )
        }

        override suspend fun state(): OfflineState = state

        override suspend fun addSet(context: String, quality: String, title: String, artist: String): SetProgress =
            state.sets.first()

        override suspend fun removeSet(kind: String, id: String) = Unit

        override suspend fun removeDownload(key: String) = Unit

        override suspend fun retry(key: String) = Unit

        override suspend fun retryAll() = Unit

        override suspend fun pause() = Unit

        override suspend fun resume() = Unit

        override suspend fun setQuality(quality: String) = Unit

        override suspend fun setOfflineOnly(onlyDownloaded: Boolean) = Unit

        override suspend fun setPlayCacheCap(bytes: Long) = Unit

        override suspend fun clearPlayCache() = Unit

        override suspend fun downloads(): ListHandle =
            throw UnsupportedOperationException("the capture paints the section, not the list")

        override suspend fun facts(keys: List<String>): List<DownloadEntry> =
            entries.filter { it.key in keys }

        override suspend fun playCacheBytes(): Long = 12L * 1024 * 1024

        override suspend fun subscribe(listener: OfflineListener) {
            this.listener = listener
            listener.onProgress(state)
        }
    }
}

/** One queued set with two failures, and a device holding 194 of 200 titles. */
private fun deviceState(): OfflineState = OfflineState(
    version = 7uL,
    state = "Téléchargement en cours",
    paused = false,
    sets = listOf(
        SetProgress(
            context = "playlist:42",
            title = "Nuit",
            kind = "playlist",
            id = "42",
            done = 194u,
            total = 200u,
            failed = 2u,
            running = true,
            paused = false,
            failures = listOf(
                "Sinnerman — le serveur prépare ce titre (réessai dans 30 s)",
                "Wild Is the Wind — Deezer : titre indisponible (404)",
            ),
        ),
        SetProgress(
            context = "album:404",
            title = "Album complet",
            kind = "album",
            id = "404",
            done = 12u,
            total = 12u,
            failed = 0u,
            running = false,
            paused = false,
            failures = emptyList(),
        ),
        SetProgress(
            context = "show:9",
            title = "Podcast",
            kind = "show",
            id = "9",
            done = 12u,
            total = 20u,
            failed = 2u,
            running = false,
            paused = false,
            failures = listOf("Épisode 7 — le flux ne répond pas"),
        ),
    ),
    settings = OfflineSettings("OPUS_320", true, 1024L * 1024 * 1024),
    storage = StorageReport(
        downloadsBytes = 1_480uL * 1024uL * 1024uL,
        cacheBytes = 12uL * 1024uL * 1024uL,
        coversBytes = 3uL * 1024uL * 1024uL,
        tracks = 194u,
        freeBytes = 0uL,
    ),
    note = "194 titres disponibles hors ligne (1,4 Go) · 2 échecs (voir la liste)",
)
