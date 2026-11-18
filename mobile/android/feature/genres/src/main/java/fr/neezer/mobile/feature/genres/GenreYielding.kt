package fr.neezer.mobile.feature.genres

import fr.neezer.mobile.bridge.NzBridge
import uniffi.nz_ffi.PlayState
import uniffi.nz_ffi.PlayerListener
import uniffi.nz_ffi.PlayerSnapshot

/**
 * La règle 1, tenue par l'écran : **tant qu'un titre démarre, les deux travaux du
 * studio se garent.**
 *
 * Le studio entraîne et mesure sur des threads à `nice 10` (le noyau), mais une
 * machine à deux cœurs n'a pas de marge : quand la personne vient de toucher un
 * titre, le mixer passe avant l'arithmétique. L'écran écoute donc l'ÉTAT du
 * lecteur et lève le drapeau tant qu'il tamponne — le desktop fait la même chose
 * avec `current_time() < 8 s` (D16).
 *
 * Un seul abonnement pour tout le processus : le studio est une page, la coque
 * peut la rouvrir, et un second abonnement doublerait les appels au noyau.
 */
internal object GenreYielding {
    private var installed = false

    fun install(service: GenresService) {
        if (installed) return
        installed = true
        runCatching {
            val player = NzBridge.core?.player() ?: return@runCatching
            player.subscribe(object : PlayerListener {
                override fun onSnapshot(snapshot: PlayerSnapshot) {
                    // Un fil de Rust : un drapeau, pas du travail.
                    service.setYielding(snapshot.state == PlayState.BUFFERING)
                }

                override fun onQueueChanged(version: ULong) = Unit

                override fun onFault(cause: String) = Unit
            })
        }.onFailure {
            // No core, no player: the studio still works, it just cannot yield.
            installed = false
        }
    }

    /** A test's own process: forget the subscription between two fixtures. */
    fun forget() {
        installed = false
    }
}
