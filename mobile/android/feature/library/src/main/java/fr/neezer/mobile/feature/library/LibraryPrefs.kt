package fr.neezer.mobile.feature.library

import android.content.Context

/**
 * Ce que l'appareil retient du tri des favoris (une trace d'interface, pas un
 * réglage serveur : aucune route, aucune migration, le même appareil retrouve
 * son ordre au redémarrage).
 *
 * Le patron est celui de `DevicePrompts.kt` et `CarPrefs.kt` ; [SharedLibraryPrefs]
 * lit deux clés d'un petit XML quand l'écran Bibliothèque s'ouvre, et écrit en
 * `apply()`. [NoLibraryPrefs] rend la même chose sans disque, pour les tests et
 * un noyau absent.
 */
interface LibraryPrefs {
    val favoriteSort: FavoriteSort
    val favoriteDescending: Boolean
    fun saveFavoriteSort(sort: FavoriteSort, descending: Boolean)
}

/** L'implémentation réelle : SharedPreferences « library », deux clés. */
class SharedLibraryPrefs(context: Context) : LibraryPrefs {
    private val prefs = context.getSharedPreferences("library", Context.MODE_PRIVATE)
    override val favoriteSort: FavoriteSort =
        runCatching { FavoriteSort.valueOf(prefs.getString("favorites.sort", null) ?: "") }
            .getOrDefault(FavoriteSort.Position)
    override val favoriteDescending: Boolean = prefs.getBoolean("favorites.desc", false)
    override fun saveFavoriteSort(sort: FavoriteSort, descending: Boolean) {
        prefs.edit().putString("favorites.sort", sort.name)
            .putBoolean("favorites.desc", descending).apply()
    }
}

/** Une implémentation inerte, pour les tests et un noyau absent. */
object NoLibraryPrefs : LibraryPrefs {
    override val favoriteSort = FavoriteSort.Position
    override val favoriteDescending = false
    override fun saveFavoriteSort(sort: FavoriteSort, descending: Boolean) = Unit
}
