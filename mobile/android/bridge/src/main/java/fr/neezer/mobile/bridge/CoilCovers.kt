package fr.neezer.mobile.bridge

import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.ContentScale
import coil3.compose.AsyncImage

/**
 * Le chargeur de pochettes, côté Coil (audit, bloqueur 3).
 *
 * Le design system déclare le seam [fr.neezer.mobile.designsystem.NzCoverLoader] et peint la rainure
 * déterministe dessous. Ce fichier est la seule implémentation qui touche Coil : son fetcher
 * [NzCoverFetcher] (qui demande le fichier au noyau, sur la voie « visible d'abord ») et AUCUN cache
 * réseau ni disque de Coil — [NeezerImages] les coupe, le fichier est déjà dans `nz-cache` (300 Mo,
 * LRU).
 *
 * **Pourquoi le pont vers le seam n'est pas ici.** `:designsystem` dépend de `:bridge`
 * (`api(project(":bridge"))` : ses lignes dessinent des `TrackRow` UniFFI), donc `:bridge` ne peut
 * pas implémenter une interface de `:designsystem` sans créer un cycle de modules. Le pont vit à la
 * racine de composition, `MainActivity`, qui voit les deux — et c'est là qu'un
 * `CompositionLocalProvider` appartient.
 *
 * `contentDescription = null` : la pochette est décorative partout où elle est dessinée (le titre
 * est lu par la ligne à côté), et `CoverImage` l'est aussi.
 */
object CoilCovers {
    @Composable
    fun Image(url: String, sizePx: Int) {
        AsyncImage(
            model = NzCoverRequest(url = url, sizePx = sizePx),
            contentDescription = null,
            contentScale = ContentScale.Crop,
            modifier = Modifier.fillMaxSize(),
        )
    }
}
