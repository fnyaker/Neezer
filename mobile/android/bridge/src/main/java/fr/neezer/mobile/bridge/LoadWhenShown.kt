package fr.neezer.mobile.bridge

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue

/** Aucune clé encore exécutée — `null` est une clé valide (la recherche sans terme). */
private val UNSET = Any()

/**
 * Exécute [load] quand la surface devient visible POUR LA PREMIÈRE FOIS, puis à
 * chaque changement de [key] tant qu'elle est visible — jamais parce qu'elle
 * redevient visible.
 *
 * Les quatre racines d'onglet vivent dans un pager qui garde ses pages composées
 * (lot SB2) : la page quittée n'est plus disposée, donc son `LaunchedEffect`
 * d'origine ne se rejoue pas au retour. Mais les pages sont composées dès le
 * lancement, visibles ou non : sans cette porte, une installation neuve lancerait
 * les quatre écrans (quatre lectures sur la voie User) avant d'avoir peint
 * l'accueil (règle 1).
 *
 * `remember` et non `rememberSaveable` : à la mort du process les modèles
 * meurent aussi (`remember`), donc le nouveau modèle DOIT recharger.
 */
@Composable
fun LoadWhenShown(visible: Boolean, key: Any? = null, load: () -> Unit) {
    var fired by remember { mutableStateOf<Any?>(UNSET) }
    LaunchedEffect(visible, key) {
        if (!visible || fired == key) return@LaunchedEffect
        fired = key
        load()
    }
}
