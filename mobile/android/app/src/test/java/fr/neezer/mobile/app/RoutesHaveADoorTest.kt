package fr.neezer.mobile.app

import java.io.File
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * La garde de l'audit (décision 13) : deux règles, écrites une fois, qui échouent
 * sur les défauts qu'un CI vert a laissés passer.
 *
 * 1. **Chaque route a une porte dans l'app**, sauf la liste `LINK_ONLY` — chaque
 *    exception porte sa raison, donc ajouter une route sans porte se voit dans le
 *    diff.
 * 2. **Aucun composable lié par `NavGraph` ne dessine `NzPlaceholder`** : c'est le
 *    défaut des podcasts (deux surcharges, le stub gagnait), et le seul qu'un test
 *    de capture par écran ne peut pas voir.
 *
 * Lecture de source plutôt que réflexion : un test qui instancie les routes ne dit
 * rien de ce qui les ATTEINT, et `Routes` vit dans le module de la coquille.
 */
class RoutesHaveADoorTest {

    /** Les routes sans porte propre, par conception — chacune avec sa raison. */
    private val linkOnly = mapOf(
        "Invite" to "un lien /invite/<code> (AppGraph)",
        "Import" to "un lien partagé ou le « + » de la Bibliothèque (AppGraph, M9)",
        "SharedPlaylist" to "un lien public /shared/playlist/<id>",
        "ReleaseShelf" to "ouvert depuis Nouveautés",
        "Party" to "un lien /party/<id> ; l'hôte passe par le lecteur (M5)",
        "RemoteClaim" to "un lien /rc/<token>",
        "History" to "un onglet de la Bibliothèque (LibraryLeaf.History)",
        "Lan" to "monté dans Réglages (LanSettingsSection)",
        "PeerSets" to "le second onglet de Téléchargements",
        "Show" to "ouvert depuis la liste des podcasts et par lien",
        "Album" to "ouvert depuis une carte ou un lien",
        "Artist" to "ouvert depuis une carte ou un lien",
        "Credits" to "ouvert depuis un menu de titre",
        "Playlist" to "ouvert depuis une carte ou un lien",
        "Search" to "un onglet et un lien",
        "Recap" to "ouvert depuis l'accueil et la Bibliothèque",
    )

    private val root: File = File(System.getProperty("nz.repoRoot") ?: ".")
    private val app = File(root, "mobile/android/app/src/main/java/fr/neezer/mobile/app")

    private fun sources(): List<File> = File(root, "mobile/android")
        .walkTopDown()
        .filter { it.isFile && it.extension == "kt" && "/build/" !in it.path }
        .toList()

    @Test
    fun everyRouteHasADoor() {
        val routes = Regex("""data (?:object|class) (\w+)""")
            .findAll(File(app, "NavGraph.kt").readText().substringAfter("object Routes {"))
            .map { it.groupValues[1] }
            .toList()
        assertTrue("aucune route trouvée : le test ne garde rien", routes.size > 15)

        // A door is something that REACHES the route. Three ways to reach it:
        //  - another file names the route (`Routes.X`, `AppLink.X`);
        //  - another file CALLS the entry composable (`XEntry(`), not merely
        //    declares it — measured on 01e7367e, `fun SetupEntry(` in the entry's
        //    own file made the literal rule pass Setup, the very defect the audit
        //    found;
        //  - the shell (NavGraph.kt) names it OUTSIDE its `composable<Routes.X>`
        //    binding — the tab's `open(tab)` and `isOn(tab)`, which is how
        //    Podcasts, Recherche and Bibliothèque are reached. The binding itself
        //    is what this rule checks against, so counting it would say nothing.
        val doors = sources().filterNot { it.name == "NavGraph.kt" }
        for (route in routes) {
            val hasDoor = doors.any { file ->
                val text = file.readText().replace(Regex("fun ${route}Entry\\("), "")
                text.contains("Routes.$route") ||
                    text.contains("AppLink.$route") ||
                    text.contains("${route}Entry(")
            } || File(app, "NavGraph.kt").readLines().any { line ->
                // …but NOT the binding itself (`composable<Routes.X>`, which is what
                // this rule checks against) and NOT the title mapping
                // (`Routes.X::class.qualifiedName`), which names the route to label
                // the bar without ever opening it — measured on 01e7367e, where that
                // line was Setup's only "door".
                line.contains("Routes.$route") &&
                    !line.contains("composable<Routes.$route>") &&
                    !line.contains("::class.qualifiedName")
            }
            assertTrue(
                "la route $route n'a aucune porte dans l'application",
                hasDoor || linkOnly.containsKey(route),
            )
        }
    }

    @Test
    fun everyBoundComposableIsARealScreen() {
        val bound = Regex("""composable<[\w.]+> *\{[^}]*?(\w+Entry)\(""")
            .findAll(File(app, "NavGraph.kt").readText())
            .map { it.groupValues[1] }
            .toSet()
        assertTrue("le graphe n'a pas été lu", bound.size > 15)

        for (entry in bound) {
            val declarations = sources().filter { it.readText().contains("fun $entry(") }
            assertTrue("le composable $entry lié par NavGraph est introuvable", declarations.isNotEmpty())
            for (file in declarations) {
                // A stub is a placeholder and NOTHING ELSE: the podcasts defect was
                // an entry whose whole body was NzPlaceholder. A real screen may
                // draw one as an honest "this device cannot" branch (PartyEntry,
                // no native core), which is rule 3's "a failure says why" — so the
                // guard asks the file for another screen composable.
                val text = file.readText()
                val drawsRealScreen = Regex("""\b\w+Screen\(""").containsMatchIn(text)
                assertFalse(
                    "le composable $entry lié par NavGraph dessine encore un NzPlaceholder (${file.name})",
                    text.contains("NzPlaceholder") && !drawsRealScreen,
                )
            }
        }
    }
}
