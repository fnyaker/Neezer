package fr.neezer.mobile.feature.genres

/**
 * La copie française du studio (lot MB18) — un seul fichier, pour qu'aucune
 * phrase ne soit écrite deux fois et qu'une relecture de la langue se fasse ici.
 *
 * Rien ici ne décide : ce sont les mots de `routes/Genres.svelte`, de
 * `GenreLibrary.svelte`, de `GenreTree.svelte`, de `GenreTagSheet.svelte`,
 * `GenreBulkSheet.svelte` et `GenreExtractor.svelte`, portés tels quels (pas
 * d'emoji, pas de « OK »).
 */
internal object GenresCopy {
    // -- les onglets ---------------------------------------------------------
    const val TAB_TAG = "Étiquetage"
    const val TAB_LIBRARY = "Bibliothèque"
    const val TAB_TREE = "Arbre"
    const val TAB_MODEL = "Modèle"

    val TABS = listOf(TAB_TAG, TAB_LIBRARY, TAB_TREE, TAB_MODEL)

    // -- l'étiquetage --------------------------------------------------------
    const val TITLE = "Genres"
    const val LOADING = "Chargement du studio…"
    const val NO_CANDIDATES = "Plus rien à étiqueter"
    const val NO_CANDIDATES_BODY =
        "Tout ce qui a un genre est étiqueté. Étiqueter un titre depuis sa fiche en ajoute ici."
    const val SORT_PLAYS = "Les plus écoutés"
    const val SORT_ACTIVE = "Les moins sûrs"
    const val RANKED_HINT = "Les neuf genres les plus probables pour ce titre."
    const val SEARCH_HINT = "Chercher un genre"
    const val UNDO = "Annuler"
    const val SKIP = "Passer"
    const val BACK = "Précédent"
    const val LABEL_SENT = "étiqueté"
    const val TAGGED_COUNT = "titres étiquetés"
    const val PREVIEW = "Écouter un extrait"
    const val PREVIEW_STOP = "Arrêter l'extrait"

    fun labelled(title: String, tag: String) = "« $title » : $tag"
    fun undone(title: String) = "Annulé : « $title »"
    fun countLabel(count: Int, one: String, many: String) =
        "$count ${if (count > 1) many else one}"

    // -- la bibliothèque -----------------------------------------------------
    const val LIBRARY_TITLE = "Votre vocabulaire"
    const val LIBRARY_BODY =
        "Le studio a semé les genres du moteur. Ajoutez les vôtres : un genre " +
            "s'étiquette, il ne se tape pas."
    const val NEW_TAG = "Nouveau genre"
    const val EDIT_TAG = "Renommer"
    const val DELETE_TAG = "Supprimer"
    const val DEFAULTS = "Genres du moteur"
    const val EMBED = "Calculer les empreintes"
    const val EMBED_FORCE = "Recalculer tout"
    const val EMBED_BODY =
        "Chaque titre archivé sans empreinte est décodé une fois. C'est le travail " +
            "long du studio : il cède la place à ce que vous écoutez."
    const val EMBED_RUNNING = "Calcul des empreintes"
    const val EMBED_DONE = "Empreintes à jour"
    const val EMBED_FAILURES = "Échecs"
    const val EXTRACTOR = "Extracteur"
    const val EXTRACTOR_TEST = "Vérifier l'extracteur"
    const val EXTRACTOR_CHECKING = "Vérification…"
    const val EXTRACTOR_OK = "L'extracteur lit les titres correctement."
    const val EXTRACTOR_BROKEN = "L'extracteur ne répond pas correctement."
    const val NO_TAG_NAME = "Un genre a besoin d'un nom"

    // -- l'arbre ------------------------------------------------------------
    const val TREE_TITLE = "Qui est sous qui"
    const val TREE_BODY =
        "Un genre peut être sous plusieurs parents. Chaque écriture renvoie l'arbre " +
            "entier : l'écran peint la réponse du serveur, jamais sa supposition."
    const val TREE_SEARCH = "Chercher dans l'arbre"
    const val TREE_UNDO = "Annuler"
    const val TREE_RESET = "Arbre d'origine"
    const val TREE_ADD = "Ranger sous…"
    const val TREE_CUT = "Couper ce lien"
    const val TREE_CREATE = "Créer un genre ici"
    const val TREE_DEFAULT = "Arbre d'origine, aucune modification"
    const val TREE_VERSION = "version"
    const val TREE_NODES = "genres"
    const val TREE_EDGES = "liens"
    const val ROOT = "racine"
    const val MULTI_PARENT = "aussi sous"

    // -- le modèle ----------------------------------------------------------
    const val MODEL_TITLE = "Le modèle"
    const val MODEL_NONE =
        "Aucun modèle : le serveur classe avec ses règles et vos étiquettes. " +
            "Entraînez-en un ci-dessous."
    const val MODEL_READY = "en service"
    const val MODEL_UNUSABLE = "illisible par le serveur : il l'ignore"
    const val MODEL_SEND = "Envoyer au serveur"
    const val MODEL_DISABLE = "Désactiver"
    const val MODEL_RELABEL = "Reclasser maintenant"
    const val MODEL_RELABEL_RUNNING = "La bibliothèque se reclasse"
    const val MODEL_RELABEL_DONE = "Dernier reclassement"
    const val TRAIN = "Entraîner"
    const val TRAIN_STOP = "Arrêter"
    const val TRAIN_AGAIN = "Réentraîner"
    const val TRAIN_NEED = "Il faut au moins deux genres de cinq titres étiquetés."
    const val SELF_TRAIN = "Apprendre aussi des titres non étiquetés"
    const val SELF_TRAIN_BODY =
        "Le modèle étiquette lui-même les titres dont il est sûr et s'en sert ; la " +
            "seconde tête ne part que si elle fait mieux sur des titres qu'elle n'a pas vus."
    const val READY_TITLE = "Avant d'entraîner"
    const val READY_BODY = "Titres étiquetés par genre. Cinq au minimum pour être appris."
    const val HEALTH = "Verdict"
    const val PER_CLASS = "Par genre"
    const val CONFUSIONS = "Confusions"
    const val CLEANUP = "Ce qui a été écarté"
    const val MEASURE = "Mesurer les titres étiquetés"
    const val MEASURE_BODY =
        "La mesure locale décode chaque titre archivé et n'envoie que des nombres : " +
            "le serveur décide. Reprend où elle s'est arrêtée."
    const val MEASURE_STOP = "Arrêter la mesure"
    const val CONSTRUCTION = "Construction"
    const val TRAIN_MODES = "Capacité"
    const val MODE_AUTO = "Automatique"
    const val MODE_LINEAR = "Rapide"
    const val MODE_DEEP = "Approfondi"
    const val MODE_DEEP2 = "Maximum"

    fun modeDescription(mode: String) = when (mode) {
        MODE_LINEAR -> "Tête linéaire. Une seconde. Le bon choix par défaut."
        MODE_DEEP -> "Réseau 256, une couche cachée. Quelques secondes."
        MODE_DEEP2 -> "Réseau 2×512, deux couches cachées. Plusieurs minutes."
        else -> "Tête linéaire, et un réseau seulement s'il fait mieux sur des " +
            "titres qu'il n'a pas vus."
    }

    fun balanced(percent: Int) = "$percent % sur des titres qu'il n'a pas vus"
    fun examples(count: Int) = countLabel(count, "exemple", "exemples")
    fun classes(count: Int) = countLabel(count, "genre", "genres")
    fun artists(count: Int) = countLabel(count, "artiste", "artistes")
    fun labelledTracks(count: Int) = countLabel(count, "titre étiqueté", "titres étiquetés")
    fun measured(done: Int, total: Int, failed: Int) =
        "$done / $total mesurés" + if (failed > 0) " · $failed échec(s)" else ""

    fun relabelRunning(scanned: Int, total: Int) = "$scanned / $total titres"
    fun relabelDone(scanned: Int, changed: Int) = "$scanned titres vus, $changed verdicts changés"

    // -- la feuille d'étiquette d'un titre ----------------------------------
    const val SHEET_TITLE = "Étiqueter le genre"
    const val SHEET_NONE = "Aucun genre"
    const val SHEET_REMOVE = "Retirer le genre"
    const val SHEET_WEARS = "porte"

    // -- la feuille de lot --------------------------------------------------
    const val BULK_TITLE = "Donner un genre"
    const val BULK_REVIEW =
        "Un désaccord arrive décoché : seuls les titres cochés portent le genre."
    const val BULK_APPLY = "Étiqueter"
    const val BULK_EMPTY = "Aucun titre à étiqueter ici."
    const val BULK_SAME = "déjà ce genre"
    const val BULK_OTHER = "porte un autre genre"
    const val BULK_MODEL = "le modèle pense autre chose"
    const val BULK_NO_VECTOR = "sans empreinte"
    const val BULK_UNJUDGED = "non jugés"
    const val BULK_MISSING = "hors bibliothèque"

    fun bulkApplied(applied: Int) = countLabel(applied, "titre étiqueté", "titres étiquetés")
    fun bulkMissing(missing: Int) =
        countLabel(missing, "titre absent de la bibliothèque", "titres absents de la bibliothèque")

    // -- les états -----------------------------------------------------------
    const val OFFLINE_COPY = "Hors ligne · version mémorisée"
    const val RETRY = "Réessayer"
    const val ADMIN_ONLY = "Réservé aux administrateurs"
    const val ADMIN_ONLY_BODY =
        "Le studio des genres décide comment toute la bibliothèque est classée : " +
            "seul un administrateur peut l'ouvrir."
    const val CORE_MISSING = "Le noyau natif n'a pas pu être chargé."
    const val ACTION_FAILED = "Action impossible"

    /** Ce que coûte un écran, écrit à côté de l'assertion qui le compte. */
    const val BUDGET_TAG = "2 requêtes à froid (statut + candidats), 0 à chaud"
}
