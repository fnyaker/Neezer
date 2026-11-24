package fr.neezer.mobile.feature.settings

import fr.neezer.mobile.bridge.NzNavigator
import uniffi.nz_ffi.AppLink

/**
 * The shell, recorded: a door that opens is a counter that moves.
 *
 * M2 declared this double inside `AccountDoorsTest`; M7 moved it here, unchanged
 * except for its own `genres` counter, because two test files now share it
 * (`AccountDoorsTest` and `VizGenresDoorTest`). The interface's new members have
 * defaults, so a test only overrides the doors it proves.
 */
internal class RecordingNavigator : NzNavigator {
    var logins = 0
    var setups = 0
    var genres = 0

    override fun go(link: AppLink) {}
    override fun back() {}
    override fun openPlayer() {}
    override fun openQueue() {}

    override fun openLogin() {
        logins++
    }

    override fun openSetup() {
        setups++
    }

    override fun openGenres() {
        genres++
    }
}
