package fr.neezer.mobile.feature.home

import uniffi.nz_ffi.ArtistCardData
import uniffi.nz_ffi.DiscoveryStateData
import uniffi.nz_ffi.DiscoveryStepData
import uniffi.nz_ffi.EnergyStepData
import uniffi.nz_ffi.FlowNextData
import uniffi.nz_ffi.FlowStateData
import uniffi.nz_ffi.HeroData
import uniffi.nz_ffi.HomeData
import uniffi.nz_ffi.MoodCardData
import uniffi.nz_ffi.RecapCardData
import uniffi.nz_ffi.ReleaseCardData
import uniffi.nz_ffi.ReleasesCardData
import uniffi.nz_ffi.ResumeCardData
import uniffi.nz_ffi.TrackRow

/**
 * The fixture the model's tests and the screenshots share: a home a real server
 * would send, in the real records — so a screenshot proves the SCREEN, not a
 * mock shape that drifted from the boundary.
 */
object HomeFixtures {
    fun row(title: String, position: Int = 0): TrackRow = TrackRow(
        key = "row-$position",
        id = "id-$position",
        title = title,
        artists = "Ascenseur",
        album = "Le matin",
        albumId = "501",
        durationS = (180 + position).toUInt(),
        cover = "/api/cover/501",
        explicit = false,
        liked = false,
        disliked = false,
        unavailable = false,
        downloaded = false,
        source = "deezer",
        position = position.toUInt(),
    )

    private fun energySteps(): List<EnergyStepData> = listOf(
        EnergyStepData(name = "Libre", value = null, hint = "Le Flow suit votre écoute"),
        EnergyStepData(name = "Très calme", value = 0.1, hint = "Doux, posé"),
        EnergyStepData(name = "Calme", value = 0.3, hint = "Tranquille"),
        EnergyStepData(name = "Modéré", value = 0.5, hint = "Entre les deux"),
        EnergyStepData(name = "Énergique", value = 0.7, hint = "Ça bouge"),
        EnergyStepData(name = "Intense", value = 0.9, hint = "À fond"),
    )

    private fun discoverySteps(): List<DiscoveryStepData> = listOf(
        DiscoveryStepData("Proche", 0.0, "Des titres qui sonnent comme ce que vous écoutez"),
        DiscoveryStepData("Voisin", 0.25, "Les artistes proches de vos favoris"),
        DiscoveryStepData("Exploration", 0.5, "Des genres voisins, des artistes à deux pas"),
        DiscoveryStepData("Aventure", 0.75, "Loin de vos habitudes"),
    )

    private fun flowSteps(): List<DiscoveryStepData> = listOf(
        DiscoveryStepData("Prudent", 0.0, "Presque uniquement ce que vous écoutez déjà."),
        DiscoveryStepData("Proche", 0.2, "Votre bibliothèque et ses voisins immédiats."),
        DiscoveryStepData("Voisin", 0.4, "Des artistes proches de vos favoris."),
        DiscoveryStepData("Exploration", 0.6, "Des genres voisins, des artistes à deux pas."),
        DiscoveryStepData("Aventure", 0.8, "Loin de vos habitudes."),
        DiscoveryStepData("Large", 1.0, "Le plus d'inconnus possible, sans boussole."),
    )

    fun flow(live: Boolean = true, waiting: Boolean = false): FlowStateData = FlowStateData(
        live = live,
        energy = 0.3,
        energyName = "Calme",
        styles = listOf("Techno"),
        level = 0.4,
        aim = "Techno · Voisin",
        aimShort = "Techno · Voisin",
        upNext = if (waiting) null else FlowNextData("row-1", "Suite", "Ascenseur"),
        waiting = waiting,
    )

    fun home(): HomeData = HomeData(
        hero = HeroData(
            name = "Nadia",
            energy = 0.3,
            energyName = "Calme",
            energySteps = energySteps(),
            discoveryName = "Voisin",
            flowLive = true,
        ),
        resume = ResumeCardData(
            key = "row-0",
            id = "id-0",
            title = "Ouverture",
            artists = "Ascenseur",
            album = "Le matin",
            cover = "/api/cover/501",
            durationS = 210u,
            positionS = 42.0,
        ),
        releases = ReleasesCardData(
            count = 3u,
            unseen = 2u,
            pending = 1u,
            releases = listOf(
                ReleaseCardData("501", "Le matin", "/api/cover/501", "Ascenseur", 0u),
                ReleaseCardData("502", "Un titre", "/api/cover/502", "June", 2u),
            ),
            tracks = listOf(row("Ouverture", 0), row("Suite", 1), row("Single", 2)),
        ),
        artists = listOf(
            ArtistCardData("a-1", "Ascenseur", "/api/cover/a-1", 12u, 40u),
            ArtistCardData("a-2", "June", null, 3u, 1u),
            ArtistCardData("a-3", "Nina", null, 8u, 9u),
        ),
        reco = listOf(row("Pour toi 1", 0), row("Pour toi 2", 1), row("Pour toi 3", 2)),
        recap = RecapCardData(year = 2025, listens = 1204u, minutes = 3890u, partial = false),
        moods = listOf(
            MoodCardData("calme", "Calme", "Doux et posé", 40u, true, "/api/cover/501"),
            MoodCardData("fete", "Fête", "Ça bouge", 32u, true, "/api/cover/502"),
        ),
        flow = flow(),
        discovery = DiscoveryStateData(
            level = 0.4,
            name = "Voisin",
            hint = "Les artistes proches de vos favoris",
            steps = discoverySteps(),
            flowSteps = flowSteps(),
        ),
        failures = 0u,
    )

    /** The genres the chips draw (the fake server's own `/flow/styles`). */
    fun styles(): List<uniffi.nz_ffi.FlowStyleData> = listOf(
        uniffi.nz_ffi.FlowStyleData("Techno", null, 10u, 12u),
        uniffi.nz_ffi.FlowStyleData("House", null, 4u, 4u),
        uniffi.nz_ffi.FlowStyleData("Électronique", null, 0u, 16u),
        uniffi.nz_ffi.FlowStyleData("Detroit", "Techno", 2u, 2u),
    )

    /** The same home with nothing in it: the "bibliothèque vide" card. */
    fun empty(): HomeData = home().copy(
        releases = null,
        artists = emptyList(),
        reco = emptyList(),
        moods = emptyList(),
        recap = null,
        resume = null,
    )
}
