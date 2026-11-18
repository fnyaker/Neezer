package fr.neezer.mobile.feature.discovery

import java.text.Normalizer

/**
 * This lot's French copy and the two formats the SPA keeps in `lib/format.js`
 * and `lib/genretree.js`.
 *
 * The Récap's own words are NOT here: they are `lib/recap.js`'s, decided in
 * `nz-core::discovery::recap` (one source of truth, tested against the SPA's own
 * node suite) and carried across as ready sentences.
 */
object DiscoveryCopy {
    /** The page's title, said once: the shell's top bar names the route. */
    const val RELEASES_LEAD =
        "Les sorties récentes de tes artistes favoris, de ceux que tu écoutes le plus et de quelques artistes très proches d'eux."

    const val ALL_RELEASES = "Toutes les sorties"
    const val ALL_RELEASES_HINT = "Regroupées par sortie, la plus récente d'abord."
    const val NOTHING_NEW_TITLE = "Rien de neuf pour l'instant"
    const val NOTHING_NEW_BODY =
        "Ajoute des artistes à tes favoris : leurs prochaines sorties apparaîtront ici."
    const val SHELF_EMPTY_ARTISTS =
        "Ajoute des artistes à tes favoris : leurs prochaines sorties apparaîtront ici."
    const val SHELF_EMPTY_DISCOVERY =
        "Le serveur proposera des artistes proches de tes favoris dès qu'il en trouvera."
    const val SHELF_FAILED = "Cette sélection n'a pas pu être chargée."
    const val OFFLINE_KEPT = "Hors ligne · version mémorisée"
    const val RÉCAP_OFFLINE_TITLE = "Ton récap a besoin du serveur"
    const val RÉCAP_INVALID_TITLE = "Cette année n'existe pas dans ton récap"
    const val RÉCAP_INVALID_BODY =
        "Le lien est peut-être incomplet, ou l'année est hors de l'historique de ce serveur."
    const val RÉCAP_NO_YEARS = "Ton récap apparaîtra quand tu auras écouté quelques titres."
    const val EXPLORER_TITLE = "Parcourir par style"
    const val EXPLORER_LEAD = "Vos styles les plus écoutés"
    const val ALL_STATIONS = "Toutes les stations"
    const val STATIONS_EMPTY = "Aucun style analysé ne correspond à votre bibliothèque."
    const val STYLE_HINT =
        "Choisissez un niveau, puis « Découvrir » : le serveur cherche des playlists Deezer dans ce style et vérifie leurs titres."
}

/**
 * `genretree.js#flat`: lowercase, accents removed, letters and digits only —
 * so "Électro" is `electro` and "Disco / funk" is `discofunk`, the same key the
 * SPA's style search compares.
 */
fun foldGenre(text: String): String {
    val decomposed = Normalizer.normalize(text, Normalizer.Form.NFD)
    val out = StringBuilder(decomposed.length)
    for (ch in decomposed) {
        if (ch.code in 0x0300..0x036F) continue
        val lower = ch.lowercaseChar()
        if (lower in 'a'..'z' || lower in '0'..'9') out.append(lower)
    }
    return out.toString()
}

/**
 * `lib/format.js#duration`: `m:ss`, or `h:mm:ss` once an hour — never a bare
 * "92:15" (a podcast episode and a long album both run past an hour).
 */
fun discDuration(seconds: Long): String {
    val total = seconds.coerceAtLeast(0)
    val hours = total / 3600
    val minutes = (total % 3600) / 60
    val secs = total % 60
    return if (hours > 0) {
        "$hours:${minutes.toString().padStart(2, '0')}:${secs.toString().padStart(2, '0')}"
    } else {
        "$minutes:${secs.toString().padStart(2, '0')}"
    }
}

/** "12 titres" / "1 titre" for the shelf's own facts. */
fun discTracks(count: Int): String = "$count titre${if (count > 1) "s" else ""}"

/** "3 sorties" / "1 sortie". */
fun discReleases(count: Int): String = "$count sortie${if (count > 1) "s" else ""}"
