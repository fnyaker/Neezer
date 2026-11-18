package fr.neezer.mobile.feature.home

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.nz_ffi.FlowStyleData

/**
 * `StylePicker.svelte#styleFamilies`' rule, on the JVM: the home's chips and
 * Explorer's tiles must name the same styles, and a family that holds nothing is
 * not a chip.
 */
class HomeStylesTest {

    private fun style(name: String, parent: String?, branch: UInt, tracks: UInt = 0u) =
        FlowStyleData(name = name, parent = parent, tracks = tracks, branch = branch)

    @Test
    fun aFamilyIsARootWithTitlesAndElectronicsIsSplitIntoItsChildren() {
        val rows = listOf(
            style("Techno", null, 40u),
            style("House", null, 10u),
            style("Électronique", null, 60u),
            style("Ambient", "Électronique", 20u),
            style("Vide", null, 0u),
        )
        val families = styleFamilies(rows).map { it.name }

        assertEquals(listOf("Techno", "Ambient", "House"), families)
        assertTrue("a family that holds nothing is not a chip", !families.contains("Vide"))
        assertTrue(
            "Électronique itself is too wide to be one chip",
            !families.contains("Électronique"),
        )
    }

    @Test
    fun theBiggestBranchComesFirstThenTheNames() {
        val rows = listOf(
            style("Zouk", null, 5u),
            style("Ambient", null, 30u),
            style("Blues", null, 5u),
        )
        assertEquals(listOf("Ambient", "Blues", "Zouk"), styleFamilies(rows).map { it.name })
    }

    @Test
    fun sixteenChipsAtMost() {
        val rows = (1..40).map { style("Style $it", null, it.toUInt()) }
        assertEquals(16, styleFamilies(rows).size)
        assertEquals("Style 40", styleFamilies(rows).first().name)
    }

    @Test
    fun aFamilysSubGenresAreTwoLevelsDeepAndNeverEmptyBranches() {
        val rows = listOf(
            style("Techno", null, 40u),
            style("Detroit", "Techno", 10u),
            style("Dub", "Detroit", 4u),
            style("Vide", "Techno", 0u),
        )
        assertEquals(listOf("Detroit", "Dub"), styleSubs(rows, "Techno").map { it.name })
    }
}
