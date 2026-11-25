package fr.neezer.mobile.bridge

import uniffi.nz_ffi.AppLink

/**
 * What a screen may ask of the shell (docs/agents/mobile.md, MB0's contract).
 *
 * One interface, held by `:app`'s `AppGraph`: a feature module navigates through
 * it and never imports a `NavController`, so a deep link and a tap land on the
 * same route and the phone and the wide shell share one implementation.
 *
 * Back goes to the SCREEN, not the route (webapp.md's rule): the implementation
 * is the only place that decides what "back" means.
 */
interface NzNavigator {
    /** Opens [link]'s route, or says why it cannot. */
    fun go(link: AppLink)

    /** Back to the screen the person came from. */
    fun back()

    /** The full-screen player, as a sheet on a phone and a column on a tablet. */
    fun openPlayer()

    /** The queue editor, scrolled to the current track. */
    fun openQueue()

    /**
     * The full-screen visualiser (MB21), opened from « Réglages → Animations » —
     * the SPA's own door (`VisualizerSettings.svelte#openScreen`).
     *
     * A DEFAULT, and it does nothing: an interface a dozen lots' test doubles
     * implement must not gain an abstract member for one screen. `:app`'s
     * `AppGraph` overrides it; every other implementation keeps compiling.
     */
    fun openViz() {}

    /**
     * L'arrivée sur l'accueil APRÈS une connexion : les écrans de préparation
     * sortent de la pile, sinon « retour » depuis l'accueil ramène sur le
     * formulaire de connexion.
     *
     * Une méthode par DÉFAUT, comme [openViz] : les doublures de test des autres
     * modules n'ont pas à l'implémenter.
     */
    fun openHomeAfterSignIn() {}

    /**
     * La session est finie (ou la personne s'est déconnectée) : la connexion,
     * le serveur conservé, la pile vidée.
     *
     * Une méthode par DÉFAUT, comme [openViz] : les doublures de test des autres
     * modules n'ont pas à l'implémenter.
     */
    fun openLogin() {}

    /**
     * « Changer de serveur » : l'écran serveur, sans rien effacer avant que la
     * personne ait validé une nouvelle adresse.
     *
     * Une méthode par DÉFAUT, comme [openViz].
     */
    fun openSetup() {}

    /**
     * La préparation du compte (`#/onboarding` du SPA) : un compte connecté qui
     * n'a pas terminé sa préparation y atterrit au lieu de la coquille vide.
     *
     * Une méthode par DÉFAUT, comme [openViz] : les doublures de test des autres
     * modules n'ont pas à l'implémenter.
     */
    fun openOnboarding() {}
}
