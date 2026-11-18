package fr.neezer.mobile.feature.trackactions

import uniffi.nz_ffi.MenuAction
import uniffi.nz_ffi.MenuEntry
import uniffi.nz_ffi.MenuItem
import uniffi.nz_ffi.PlaylistChoice
import uniffi.nz_ffi.ReplaceCandidate
import uniffi.nz_ffi.TrackRow

/**
 * The records the JVM tests and the screenshots share.
 *
 * They are the REAL generated types, filled the way the core fills them, so a
 * test never asserts against a shape of its own invention.
 */
object TrackActionsFixtures {
    const val KEY = "deezer:9001"
    const val ID = "9001"
    const val PLAYLIST_ID = "11111111-1111-4111-8111-111111111111"
    const val OTHER_PLAYLIST_ID = "22222222-2222-4222-8222-222222222222"

    fun row(
        title: String = "Ouverture",
        key: String = KEY,
        id: String = ID,
        liked: Boolean = false,
        disliked: Boolean = false,
        source: String = "deezer",
        position: Int = 0,
    ): TrackRow = TrackRow(
        key = key,
        id = id,
        title = title,
        artists = "Ascenseur",
        album = "Le matin",
        albumId = "501",
        durationS = 181u,
        cover = "/api/cover/501",
        explicit = false,
        liked = liked,
        disliked = disliked,
        unavailable = false,
        downloaded = false,
        source = source,
        position = position.toUInt(),
    )

    fun item(
        label: String,
        action: MenuAction,
        icon: String = "music",
        sub: List<MenuItem> = emptyList(),
        danger: Boolean = false,
    ): MenuItem = MenuItem(label = label, icon = icon, action = action, sub = sub, danger = danger)

    /** The SPA's own list, as the builder hands it over (dividers included). */
    fun entries(liked: Boolean = false): List<MenuEntry> = listOf(
        MenuEntry.Item(item("Lire ensuite", MenuAction.PlayNext, icon = "next")),
        MenuEntry.Item(item("Ajouter à la file", MenuAction.AddToQueue, icon = "queue")),
        MenuEntry.Item(item("Ajouter à une playlist…", MenuAction.OpenPlaylistPicker)),
        MenuEntry.Divider,
        MenuEntry.Item(item("Partager…", MenuAction.Share, icon = "share")),
        MenuEntry.Item(item("Lancer la radio", MenuAction.StartRadio, icon = "radio")),
        MenuEntry.Item(
            item(
                if (liked) "Retirer des favoris" else "Ajouter aux favoris",
                MenuAction.Favorite(on = !liked),
                icon = if (liked) "heartFilled" else "heart",
            ),
        ),
        MenuEntry.Item(item("Je n'aime pas", MenuAction.Dislike(on = true), icon = "thumbsDown")),
        MenuEntry.Divider,
        MenuEntry.Item(item("Voir les crédits", MenuAction.OpenCredits, icon = "users")),
        MenuEntry.Item(
            item(
                "Aller à l'artiste",
                MenuAction.GoToArtist("aaaaaaaa-1111-4111-8111-111111111111"),
                icon = "user",
                sub = listOf(
                    item("Ascenseur", MenuAction.GoToArtist("aaaaaaaa-1111-4111-8111-111111111111"), "user"),
                    item("June", MenuAction.GoToArtist("cccccccc-1111-4111-8111-111111111111"), "user"),
                ),
            ),
        ),
        MenuEntry.Item(item("Aller à l'album", MenuAction.GoToAlbum("501"), icon = "disc")),
    )

    /** The episode's own, shorter list (`buildEpisodeMenu`). */
    fun episodeEntries(): List<MenuEntry> = listOf(
        MenuEntry.Item(item("Lire ensuite", MenuAction.PlayNext, icon = "next")),
        MenuEntry.Item(item("Ajouter à la file", MenuAction.AddToQueue, icon = "queue")),
        MenuEntry.Item(item("Partager…", MenuAction.Share, icon = "share")),
        MenuEntry.Divider,
        MenuEntry.Item(
            item(
                "Ouvrir le podcast",
                MenuAction.OpenPodcast("dddddddd-1111-4111-8111-111111111111"),
                icon = "mic",
            ),
        ),
    )

    fun choice(
        id: String = PLAYLIST_ID,
        title: String = "Route du soir",
        editable: Boolean = true,
        recent: Boolean = false,
        nbTracks: Int = 4,
        role: String? = "owner",
        owner: String? = null,
    ): PlaylistChoice = PlaylistChoice(
        id = id,
        title = title,
        cover = null,
        nbTracks = nbTracks.toUInt(),
        editable = editable,
        role = role,
        owner = owner,
        recent = recent,
        version = 2u,
    )

    fun candidate(
        key: String = "local:44444444-4444-4444-8444-444444444444",
        id: String = "44444444-4444-4444-8444-444444444444",
        title: String = "Ouverture",
        source: String = "local",
        local: Boolean = true,
    ): ReplaceCandidate = ReplaceCandidate(
        key = key,
        id = id,
        title = title,
        artists = "Ascenseur",
        album = "Le matin",
        durationS = 181u,
        cover = null,
        source = source,
        local = local,
    )

    /** What a screen says when a write was refused: the core's own French copy. */
    fun offline(): Exception = IllegalStateException("Hors ligne : réessaie quand le réseau revient")
}
