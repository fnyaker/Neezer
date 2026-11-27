package fr.neezer.mobile.feature.home

import uniffi.nz_ffi.FlowStyleData

/**
 * The style families `StylePicker.svelte#styleFamilies` builds its chips from —
 * ported word for word, because the home's chips and Explorer's tiles must name
 * the same styles:
 *
 * - the tree's ROOTS, with "Électronique" split into its children (it is too
 *   wide to be one chip);
 * - only a node that holds titles somewhere under it (`branch > 0`);
 * - the biggest branch first, alphabetically on a tie (the server's own order),
 *   and sixteen chips at most.
 *
 * Pure, so `HomeStylesTest` holds the rule on the JVM.
 */
fun styleFamilies(rows: List<FlowStyleData>): List<FlowStyleData> {
    val byParent = rows.groupBy { it.parent ?: "" }
    val roots = byParent[""].orEmpty().filter { it.name != "Électronique" }
    val children = byParent["Électronique"].orEmpty()
    return (roots + children)
        .filter { it.branch > 0u }
        .sortedWith(compareByDescending<FlowStyleData> { it.branch }.thenBy { it.name })
        .take(16)
}

/**
 * `sillon.js#STYLE_FALLBACK`, word for word: the eleven styles the SPA draws when
 * the tree holds NO measured genre.
 *
 * That is the state of a young library and of the release bench (`/flow/styles`
 * answers `{"styles": []}` because no title has been analysed yet), and the SPA
 * still shows its chips there: `StylePicker.svelte`'s `visibleFamilies =
 * families.length ? families : STYLE_FALLBACK.map(…)`. The port must do the same
 * — an empty answer is not a reason to drop the block, it is the reason for the
 * fallback — otherwise the home loses a section the reference shows (lot JH2,
 * measured on the bench server: one `GET /api/flow/styles` → `{"styles": []}`).
 *
 * They are names only: no sub-genres, no counts, and picking one is a plain
 * style choice the server resolves through the genre tree.
 */
val STYLE_FALLBACK: List<String> = listOf(
    "Techno",
    "Hard dance",
    "Hardcore",
    "Pop",
    "Hip-hop",
    "Rock",
    "Dance/EDM",
    "Drum & bass",
    "Synthwave",
    "Trance",
    "Indie",
)

/** What the chips name: the families the answer holds, or [STYLE_FALLBACK]. */
fun styleChoices(rows: List<FlowStyleData>): List<String> =
    styleFamilies(rows).map { it.name }.ifEmpty { STYLE_FALLBACK }

/**
 * The sub-genres a family opens, depth first and two levels deep, exactly as the
 * SPA's `flat(name)` walks the tree — a family's finer choice is what a second
 * tap shows.
 */
fun styleSubs(rows: List<FlowStyleData>, family: String, depth: Int = 0): List<FlowStyleData> {
    val out = mutableListOf<FlowStyleData>()
    for (row in rows.filter { (it.parent ?: "") == family }) {
        if (row.branch <= 0u) continue
        out += row
        if (depth < 2) out += styleSubs(rows, row.name, depth + 1)
    }
    return out
}
