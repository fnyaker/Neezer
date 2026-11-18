package fr.neezer.mobile.feature.audiofx

/**
 * Every sentence the effects card says, in French, taken from
 * `webapp/src/components/AudioEffects.svelte` — the same words the SPA uses,
 * because a person who used the web app must find the same controls under the
 * same names.
 *
 * The card's own headings are the top bar's title on the phone: the shell already
 * names the route, so a screen that repeats its name shows it twice
 * (`docs/agents/mobile.md`, MB16's rule).
 */
internal object FxCopy {
    const val OUTPUT_TITLE = "Sortie audio"
    const val OUTPUT_SUB =
        "Le chemin du son jusqu'à vos écouteurs, votre enceinte ou l'autoradio."

    val OUTPUT_MODES = listOf("Automatique" to "auto", "Directe" to "direct", "Traitée" to "graph")
    val OUTPUT_HINTS = mapOf(
        "auto" to
            "Traitée, et directe seulement quand le lecteur mesure que le son se hache " +
            "(un autoradio Bluetooth, parfois), le temps que ça dure.",
        "direct" to
            "Le son part tel quel vers le système : la lecture la plus robuste. " +
            "L'égaliseur et les basses sont inactifs.",
        "graph" to
            "Tout passe par le moteur audio de l'application : égaliseur, basses, " +
            "normalisation complète.",
    )

    const val PAN_TITLE = "Mono et balance"
    const val PAN_SUB =
        "Pour écouter avec un seul écouteur, une oreille ou une aide auditive : " +
        "réunissez les deux canaux, ou déplacez le son vers l'oreille qui entend. " +
        "Ces réglages ne changent que ce que vous entendez — les animations gardent " +
        "le stéréo d'origine."
    const val MONO = "Mono"
    const val MONO_HINT =
        "Réunit les deux canaux dans chaque oreille : plus rien n'est perdu d'un côté, " +
        "même avec un seul écouteur."
    const val LEFT = "Gauche"
    const val RIGHT = "Droite"
    const val CENTRE = "Centre"
    const val PAN_APPLIED = "Appliqué immédiatement, sans interrompre la lecture."

    const val AUDIO_TITLE = "Audio"
    const val AUDIO_SUB =
        "Ces effets traitent le son dans l'application. Ils sont désactivés par défaut ; " +
        "une fois activés, la lecture passe par le processeur audio."

    const val NORMALIZATION = "Normalisation du volume"
    const val NORMALIZATION_HINT =
        "Gain fixe par titre (selon sa loudness ReplayGain analysée par Deezer) pour que " +
        "tous jouent au même niveau — sans compression ni ajustement pendant la lecture."
    val NORM_LEVELS = listOf(
        "Désactivée" to "off",
        "Basse" to "low",
        "Normale" to "medium",
        "Élevée" to "high",
    )

    const val BASS = "Amélioration des basses"
    const val BASS_HINT = "Renforce le grave. Un limiteur automatique évite la saturation."

    const val EQ_TITLE = "Égaliseur"
    const val EQ_SUB =
        "Dix bandes de 31 Hz à 16 kHz (±16 dB). Un limiteur de sécurité rattrape les pics " +
        "en fin de chaîne ; le mètre montre le niveau réel de chaque bande et l'effort du " +
        "limiteur. Le risque de saturation ne s'affiche que si votre courbe relève le " +
        "signal, et reste visible jusqu'à votre prochain réglage."
    const val EQ_SWITCH = "Égaliseur 10 bandes"
    const val EQ_SWITCH_HINT = "Ajustez chaque bande (±16 dB), ou partez d'un préréglage."
    const val BUILTINS = "Intégrés"
    const val MINE = "Mes préréglages"
    const val NO_PRESETS = "Aucun préréglage enregistré."
    const val NEW_PRESET = "Nouveau préréglage"
    const val SAVE = "Enregistrer"
    const val SAVE_AS = "Enregistrer comme…"
    const val UPDATE = "Mettre à jour"
    const val DELETE = "Supprimer"
    const val CANCEL = "Annuler"
    const val PRESET_NAME = "Nom du préréglage"
    const val PRESET_PLACEHOLDER = "Nom du préréglage (ex. Casque)"

    const val METER_TITLE = "Niveau par bande"
    const val METER_PEAK = "Pic"
    const val METER_HEADROOM = "Marge"
    const val METER_LIMITER = "Limiteur"
    const val METER_PENDING = "Mesure en cours…"
    const val METER_IDLE = "La mesure démarre avec la lecture."
    const val METER_DIRECT = "Mesure indisponible : le son ne traverse pas l'égaliseur."
    const val METER_NATIVE = "Mesure indisponible avec le lecteur de l'application."

    const val TRANSITIONS_TITLE = "Transitions"
    const val TRANSITIONS_SUB =
        "Comment un titre laisse la place au suivant. Le fondu respecte la normalisation : " +
        "chaque titre garde son propre niveau pendant le chevauchement, aucun des deux ne saute."
    const val CROSSFADE = "Fondu enchaîné"
    const val CROSSFADE_HINT = "Les deux titres se superposent en fin de morceau."
    const val CROSSFADE_DURATION = "Durée"
    const val SKIP_FADE = "Adoucir les changements manuels"
    const val SKIP_FADE_HINT =
        "Suivant et précédent s'enchaînent avec un fondu plus court que celui de fin de " +
        "titre, sans la coupure sèche au milieu de la forme d'onde."
    const val SKIP_DURATION = "Durée"
    const val TRIM = "Tronquer les silences"
    const val TRIM_HINT =
        "Démarre et termine chaque titre là où le son commence et s'arrête vraiment. " +
        "Le serveur mesure le fichier une fois, après son archivage : un titre est donc " +
        "ajusté à partir de la lecture suivante."

    /** The threshold is the SERVER's measurement, not a device setting. */
    fun trimThreshold(db: Int) =
        "Le serveur mesure les bords au seuil de $db dBFS, le même pour tous les titres."

    fun reduceVolume(cut: String) = "Réduire le volume de $cut"
    fun volumeReduced(cut: String) = "Volume réduit de $cut"
    fun undo() = "Annuler"
    fun retry() = "Réessayer maintenant"
    fun retryAt(clock: String) = "Nouvel essai vers $clock, ou dès que la sortie se reconnecte."
    const val RETRY_NEXT_TRACK = "Nouvel essai au prochain titre."

    fun clip() = "Saturation"
    fun warn() = "Risque de saturation"
    fun limiterPulls(reduction: String, more: String) = "Le limiteur réduit de $reduction$more."
    fun moreSinceChange() = " de plus depuis votre réglage"
    fun overFullScale() = "Le signal dépasse 0 dBFS."

    fun saved(name: String) = "Préréglage « $name » enregistré"
    fun updated(name: String) = "Préréglage « $name » mis à jour"
    fun deleted(name: String) = "Préréglage « $name » supprimé"
}
