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
}
