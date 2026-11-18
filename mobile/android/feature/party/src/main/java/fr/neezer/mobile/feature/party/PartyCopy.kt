package fr.neezer.mobile.feature.party

/**
 * Every French sentence the two party screens say (lot MB20).
 *
 * The lines that describe the PARTY ITSELF come from Rust — the status line, a
 * guest's own words on the host's list, the audience count, the two dead-link
 * pages and the landing invitation are `nz-party`'s and the SPA's templates,
 * carried in `PartyStateData` — because the shared crate is where that rule
 * lives and one place must own it. What is here is what belongs to a SCREEN: the
 * sheet's three arguments, its buttons, the hint under the join button, and the
 * *Décalage* card's explanation.
 */
object PartyCopy {
    /** The feature's name — the door, the sheet's title, the route's title. */
    const val TITLE = "Soirée d'écoute"

    /** The sheet's subtitle before a party exists (`PartySheet.svelte`). */
    const val SUBTITLE = "La même musique, au même instant, sur chaque appareil."

    // --- the host's three arguments ------------------------------------------
    const val POINT_LINK =
        "Partagez un lien. Ceux qui l'ouvrent entendent ce que vous jouez, calé à la " +
            "milliseconde près — sans compte."
    const val POINT_ROOM = "Dans une même pièce, chaque téléphone devient une enceinte de plus."
    const val POINT_HAND = "Vous gardez la main : ils suivent vos titres, vos pauses et vos enchaînements."

    // --- the door and the buttons --------------------------------------------
    /** The player's door (`contentDescription`, and the slot's label). */
    const val DOOR = "Soirée d'écoute"
    const val START = "Démarrer la soirée"
    const val STARTING = "Démarrage…"
    const val CLOSE = "Fermer"
    const val SHARE = "Partager"
    const val COPY = "Copier"
    const val END = "Terminer la soirée"
    const val END_CONFIRM = "Confirmer : tout le monde est déconnecté"
    const val SCAN = "Scannez, ou envoyez le lien"
    const val INVITE_TEXT = "Viens écouter avec moi — même musique, même instant."
    const val INVITE_TITLE = "Soirée d'écoute"
    const val PEOPLE = "À l'écoute"
    const val NOBODY =
        "Personne pour l'instant. Dès qu'un invité ouvre le lien, il apparaît ici."

    /** The two toasts the sheet raises (`PartySheet.svelte`). */
    const val START_FAILED = "Impossible de démarrer la soirée"
    const val ENDED = "Soirée terminée"
    const val COPIED = "Lien copié"
    const val COPY_FAILED = "Copie impossible — sélectionnez le lien"

    // --- the notes a host sees -----------------------------------------------
    const val ELSEWHERE = "Cette soirée est animée depuis un autre écran : c'est lui qui donne le tempo."
    const val LOST = "Le serveur a redémarré et le lien a changé — partagez le nouveau."

    // --- the guest's landing card --------------------------------------------
    const val NAME_LABEL = "Votre prénom (visible par l'hôte)"
    const val NAME_PLACEHOLDER = "Invité"
    const val JOIN = "Rejoindre l'écoute"
    const val JOIN_HINT =
        "Le son démarre calé sur celui de l'hôte. Gardez l'écran allumé : un téléphone " +
            "verrouillé coupe le son."

    /** What « Rejoindre à nouveau » does after leaving (`Party.svelte`). */
    const val REJOIN = "Rejoindre à nouveau"

    // --- the live view -------------------------------------------------------
    const val QUIT = "Quitter la soirée"
    const val NEXT = "Ensuite"
    const val VOLUME = "Volume"
    const val TRIM = "Décalage"
    const val TRIM_LABEL = "Décalage de cet appareil"

    /** The *Décalage* card's explanation (`Party.svelte`'s `.latency`). */
    const val TRIM_HELP =
        "Cet appareil sonne en retard sur les autres (enceinte Bluetooth, barre de son) ? " +
            "Avancez-le. En avance ? Retardez-le. Réglez à l'oreille, pièce silencieuse, " +
            "deux appareils côte à côte."

    /** The OS's own figure, when it reports one. */
    fun osLatency(ms: Long): String = "Sortie audio : $ms ms, déjà compensée par Android"

    /** The shortcut the SPA only offers when the OS reports under 100 ms. */
    const val BLUETOOTH = "Bluetooth (~150 ms)"

    // --- the failure the peek can leave --------------------------------------
    const val RETRY = "Réessayer"
    const val UNSUPPORTED = "L'écoute partagée n'est pas disponible sur ce serveur."

    /** A party that was not found says so before the screen even asks. */
    const val MISSING = "Cette soirée n'existe plus."
}
