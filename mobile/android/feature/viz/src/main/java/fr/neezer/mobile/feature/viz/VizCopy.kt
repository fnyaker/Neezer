package fr.neezer.mobile.feature.viz

import fr.neezer.mobile.designsystem.NzGlyph

/**
 * Every French sentence and every vocabulary of the visualiser, in one place —
 * `routes/Viz.svelte`, `components/VisualizerSettings.svelte`,
 * `components/FlashWarning.svelte` and `components/EcoToggle.svelte`'s own copy.
 *
 * A screen never composes a sentence out of a stored id: the ids are the SPA's
 * (`bars`, `scope`, `cover`, `duo`, …) and the labels are the words the person
 * reads, exactly as the two Svelte files write them.
 */
internal object VizCopy {
    /** The screen's masthead (`NavGraph` already titles the route). */
    const val TITLE = "Visualiseur"

    /** `VisualizerSettings.svelte#FPS_CHOICES`, the two caps Réglages offers. */
    val FPS_CHOICES = listOf(
        Choice("30", "30 i/s", "Le plus économe", 30),
        Choice("60", "60 i/s", "Fluide", 60),
    )

    /** `lib/viz/modes.js#MODES`, minus the modes MB22 owns. */
    val MODES = listOf(
        Mode(
            id = "bars",
            label = "Barres",
            hint = "Le spectre en capsules de lumière, avec une vraie résolution dans les graves.",
        ),
        Mode(
            id = "scope",
            label = "Oscilloscope",
            hint = "Le signal lui-même, une trace par canal, déclenchée comme sur un vrai oscilloscope.",
        ),
    )

    /**
     * `lib/viz/palette.js#PALETTES`, as this screen can colour:
     * `cover` (the sleeve), `duo` (the two-tone scheme Réglages calls « Duo »)
     * and `mono`. The three fixed schemes the settings do not offer are read
     * but not shown — a stored `warm` keeps working, the pills stay at three.
     */
    val PALETTES = listOf(
        Palette("cover", "Pochette"),
        Palette("duo", "Duo"),
        Palette("mono", "Monochrome"),
    )

    /** `lib/viz/scenes/scope.js#SCOPE_ORIENTATIONS`. */
    val ORIENTATIONS = listOf(
        Choice("horizontal", "Horizontal", "Deux bandes, l'une sous l'autre", 0),
        Choice("vertical", "Vertical", "Deux colonnes, côte à côte", 1),
    )

    /** `lib/viz/scenes/scope.js#SCOPE_COLOURS`, minus `sweep` (see PALETTES). */
    val SCOPE_COLOURS = listOf(
        Choice("duo", "Deux teintes", "Gauche et droite aux deux extrémités de la palette", 0),
        Choice("mono", "Une teinte", "Les deux voies sur la couleur de base", 1),
    )

    /** The PAUSED notice, `routes/Viz.svelte`'s quiet block. */
    const val PAUSED_TITLE = "En pause"
    const val PAUSED_BODY = "L'animation reprend dès que la lecture repart."

    /** Nothing in the player at all. */
    const val IDLE_TITLE = "Rien en lecture"
    const val IDLE_BODY = "Lancez un titre depuis le lecteur."

    /** No cover, no title — the card is simply absent, the scene is the screen. */
    const val CARD_UNKNOWN = "Sans titre"

    /** Eco stops the picture; this is what the screen says instead of a canvas. */
    const val ECO_TITLE = "Mode éco actif"
    const val ECO_BODY =
        "Aucune animation : c'est le réglage qui coupe tout, y compris le visualiseur. La lecture et les indicateurs d'état continuent."

    /** `EcoToggle.svelte`'s two toasts. */
    const val ECO_ON = "Mode éco activé — aucune animation"
    const val ECO_OFF = "Mode éco désactivé"
    const val ECO_LABEL = "Mode éco (aucune animation)"
    const val ECO_TITLE_ON = "Mode éco actif — aucune animation"
    const val ECO_TITLE_OFF = "Mode éco : couper toutes les animations"

    /** The full-screen button (`routes/Viz.svelte#toggleFullscreen`). */
    const val FULLSCREEN = "Plein écran"
    const val LEAVE_FULLSCREEN = "Quitter le plein écran"

    /** A control that could not be written: the cause comes from the FFI. */
    const val SETTING_FAILED = "Réglage non enregistré"

    // --- FlashWarning.svelte -------------------------------------------------

    const val FLASH_TITLE = "Flashs débridés"
    const val FLASH_SUBTITLE = "Un stroboscope, calé sur la musique."
    const val FLASH_BODY =
        "Chaque frappe principale d'un drop devient un éclair, les grosses frappes plus fort " +
            "encore, et les caisses claires d'une montée accélèrent jusqu'au drop — jusqu'à dix " +
            "éclairs par seconde, plus intenses qu'en « Plein ». C'est au-delà du seuil de trois " +
            "par seconde que les règles d'accessibilité fixent aux lumières clignotantes."
    const val FLASH_WARN_TITLE = "Risque pour les personnes photosensibles."
    const val FLASH_WARN_BODY =
        "Des flashs répétés peuvent déclencher une crise d'épilepsie, y compris chez quelqu'un " +
            "qui n'en a jamais fait et ne se sait pas concerné. Arrêtez tout de suite en cas de " +
            "vertige, de vision troublée, de secousses musculaires ou de perte de repères."
    const val FLASH_ROOM =
        "Sur l'écran séparé, pensez aux personnes présentes : elles n'ont rien accepté."
    const val FLASH_REVERSIBLE =
        "Réversible à tout moment ici, et coupé d'office si l'appareil demande de réduire les animations."
    const val FLASH_REDUCED =
        "Votre appareil demande actuellement de réduire les animations : tant que ce réglage " +
            "système est actif, aucun flash ne sera affiché."
    const val FLASH_AGREE = "Je ne suis pas photosensible, et j'accepte ce risque pour cet appareil."
    const val FLASH_CANCEL = "Annuler"
    const val FLASH_ACCEPT = "Activer"
    const val FLASH_CLOSE = "Fermer"

    /**
     * L'avertissement du visualiseur : les phrases `FLASH_*` décrivent le niveau
     * « Débridé » du SPA (dix éclats par seconde), que ce téléphone n'a pas — les
     * mondes sont en pause. Ces deux phrases disent ce que l'écran FAIT.
     */
    const val FLASH_VIZ_SUBTITLE = "Des lumières qui suivent la musique, parfois par éclats."
    const val FLASH_VIZ_BODY =
        "Le visualiseur fait varier la lumière au rythme du morceau : des éclats suivent les " +
            "frappes, plus forts sur les grosses caisses. Rien n'est réglé au-delà de trois " +
            "éclats par seconde, mais un écran plein cadre reste une lumière qui pulse."

    /** A field of a picker: the id Rust stores, the word the person reads. */
    data class Choice(val id: String, val label: String, val hint: String, val index: Int)

    /** One scene: what it is called and what it costs (`modes.js`). */
    data class Mode(val id: String, val label: String, val hint: String)

    /** One palette source. */
    data class Palette(val id: String, val label: String)

    /** The glyph of a scene, for the segmented control's own row. */
    fun glyph(mode: String): NzGlyph = if (mode == "scope") NzGlyph.Activity else NzGlyph.Gauge
}
