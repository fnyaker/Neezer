package org.nsupysonic.app

import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.w3c.dom.Element
import java.io.File
import javax.xml.parsers.DocumentBuilderFactory

/**
 * Two contracts that live in a FILE rather than in a function, and that nothing
 * on the JVM can exercise (they need a real activity and a real WebView):
 *
 *  - MainActivity must not be RECREATED for a display or text setting: a config
 *    change it does not declare destroys the activity, `onDestroy` releases the
 *    native player and the notification keeps saying "playing" over silence (a
 *    system font size was enough);
 *  - MainActivity must call `window.__nsAppBackground` / `__nsAppForeground`,
 *    the hooks Player.svelte installs to learn the ACTIVITY is stopped (a
 *    `visibilitychange` is not reliable in a WebView that is deliberately never
 *    paused). The doc and the bench assumed it; the code did not do it.
 *
 * Same spirit as `tests/test_agent_docs.py`: it reads the file it guards. Skipped
 * (not failed) when the Gradle working directory is not the module's.
 */
class AppContractTest {

    private fun source(rel: String): File? {
        val roots = listOfNotNull(
            System.getProperty("user.dir"),
            System.getProperty("user.dir")?.let { File(it).parentFile?.path },
            System.getProperty("user.dir")?.let { File(it).parentFile?.parentFile?.path },
        )
        for (r in roots) {
            for (p in listOf(rel, "app/$rel")) {
                val f = File(r, p)
                if (f.isFile) return f
            }
        }
        return null
    }

    @Test
    fun aDisplayOrTextSettingDoesNotRecreateTheActivity() {
        val f = source("src/main/AndroidManifest.xml")
        assumeTrue("AndroidManifest.xml not found from ${System.getProperty("user.dir")}", f != null)
        val doc = DocumentBuilderFactory.newInstance().apply { isNamespaceAware = false }
            .newDocumentBuilder().parse(f)
        val activities = doc.getElementsByTagName("activity")
        var main: Element? = null
        for (i in 0 until activities.length) {
            val e = activities.item(i) as Element
            val name = e.getAttribute("android:name")
            if (name == ".MainActivity" || name == "org.nsupysonic.app.MainActivity") main = e
        }
        assumeTrue("MainActivity not in the manifest", main != null)
        val declared = main!!.getAttribute("android:configChanges").split("|").toSet()
        // The ones Android delivers for a display/locale change. Without them the
        // activity is recreated and the audio stops mid-song.
        for (change in listOf("fontScale", "density", "locale", "layoutDirection")) {
            assertTrue(
                "MainActivity must declare android:configChanges=\"$change\" (has ${declared.joinToString("|")})",
                change in declared,
            )
        }
        // ...and the ones it always had must not be dropped on the way.
        for (change in listOf("orientation", "screenSize", "uiMode")) {
            assertTrue("MainActivity lost $change from configChanges", change in declared)
        }
    }

    @Test
    fun theActivityTellsThePageWhenItStartsAndStops() {
        val f = source("src/main/java/org/nsupysonic/app/MainActivity.kt")
        assumeTrue("MainActivity.kt not found from ${System.getProperty("user.dir")}", f != null)
        val src = f!!.readText()
        // The hooks Player.svelte installs (webapp/src/components/Player.svelte).
        assertTrue(
            "MainActivity must call window.__nsAppBackground when the activity stops",
            src.contains("__nsAppBackground") && src.contains("pushAppState(true)"),
        )
        assertTrue(
            "MainActivity must call window.__nsAppForeground when it starts again",
            src.contains("__nsAppForeground") && src.contains("pushAppState(false)"),
        )
        // onStop must tell the page BEFORE the players are told: the page is still
        // running then, and it is its one chance to name the long list.
        val stop = src.substringAfter("override fun onStop()", "")
            .substringBefore("override fun onResume()")
        val tell = stop.indexOf("pushAppState(true)")
        val players = stop.indexOf("setForeground(false)")
        assertTrue("onStop must call pushAppState(true)", tell >= 0)
        assertTrue("onStop must call setForeground(false)", players >= 0)
        assertTrue(
            "onStop must tell the page BEFORE the players (pushAppState first)",
            tell < players,
        )
    }
}
