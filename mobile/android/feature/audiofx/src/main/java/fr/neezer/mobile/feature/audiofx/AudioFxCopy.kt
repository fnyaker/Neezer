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

    val OUTPUT_MODES = listOf("Automatique" to "auto", "Directe" to "direct", "Traitée" to "graph")

    const val PAN_TITLE = "Mono et balance"
    const val MONO = "Mono"
    const val LEFT = "Gauche"
    const val RIGHT = "Droite"
    const val CENTRE = "Centre"

    const val AUDIO_TITLE = "Audio"

    const val NORMALIZATION = "Normalisation du volume"
    val NORM_LEVELS = listOf(
        "Désactivée" to "off",
        "Basse" to "low",
        "Normale" to "medium",
        "Élevée" to "high",
    )

    const val BASS = "Amélioration des basses"

    const val EQ_TITLE = "Égaliseur"
    const val EQ_SWITCH = "Égaliseur 10 bandes"
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
    const val METER_IDLE = "Mesure pendant la lecture."
    const val METER_DIRECT = "Mesure indisponible : le son ne traverse pas l'égaliseur."
    const val METER_NATIVE = "Mesure indisponible avec le lecteur de l'application."

    const val TRANSITIONS_TITLE = "Transitions"
    const val CROSSFADE = "Fondu enchaîné"
    const val CROSSFADE_DURATION = "Durée"
    const val SKIP_FADE = "Adoucir les changements manuels"
    const val SKIP_DURATION = "Durée"
    const val TRIM = "Tronquer les silences"

    /** The threshold is the SERVER's measurement, not a device setting. */
    fun trimThreshold(db: Int) = "Seuil serveur · $db dBFS"

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
