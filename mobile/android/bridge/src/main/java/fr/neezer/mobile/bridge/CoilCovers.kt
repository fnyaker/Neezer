package fr.neezer.mobile.bridge

import android.content.Context
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import coil3.BitmapImage
import coil3.SingletonImageLoader
import coil3.compose.AsyncImage
import coil3.request.ImageRequest
import coil3.request.allowHardware

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

    /**
     * Les pixels d'une pochette, décodés à `sizePx` — le chemin du fond du
     * lecteur (JF), qui doit flouter l'image lui-même.
     *
     * `allowHardware(false)` est obligatoire : un bitmap lu pixel par pixel ne
     * peut pas être un `Hardware` bitmap. Le fichier vient déjà de `nz-cache`
     * (le même fetcher que l'[Image] dessinée), donc c'est un décodage, jamais
     * une requête réseau; et c'est le MÊME `SingletonImageLoader` — la règle du
     * fichier, « the one ImageLoader the app uses ».
     */
    suspend fun bitmap(context: Context, url: String, sizePx: Int): ImageBitmap? {
        val result = SingletonImageLoader.get(context).execute(
            ImageRequest.Builder(context)
                .data(NzCoverRequest(url = url, sizePx = sizePx))
                .size(sizePx)
                .allowHardware(false)
                .build(),
        )
        return (result.image as? BitmapImage)?.bitmap?.asImageBitmap()
    }
}
