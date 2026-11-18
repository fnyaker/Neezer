package fr.neezer.mobile.feature.nowplaying

import fr.neezer.mobile.designsystem.NzRowSource
import uniffi.nz_ffi.AudioSettings
import uniffi.nz_ffi.CardLineData
import uniffi.nz_ffi.CardSelectionData
import uniffi.nz_ffi.CreditsInfoData
import uniffi.nz_ffi.LyricLine
import uniffi.nz_ffi.LyricsData
import uniffi.nz_ffi.NowPlaying
import uniffi.nz_ffi.PlayState
import uniffi.nz_ffi.PlayerCommand
import uniffi.nz_ffi.PlayerSnapshot
import uniffi.nz_ffi.PositionInfo
import uniffi.nz_ffi.Quality
import uniffi.nz_ffi.Repeat
import uniffi.nz_ffi.SleepTimer
import uniffi.nz_ffi.TrackCreditsData
import uniffi.nz_ffi.TrackRow

/**
 * The fakes the JVM suites drive: one for the player, one for the core's screens,
 * both counting what they were asked — so "one request", "0 FFI calls", "one
 * command" are ASSERTIONS, not hopes (the mobile plan's rule: counts, not times).
 *
 * A fake is not a second implementation of a rule: it records the call and answers
 * a canned value. Every rule under test is the MODEL's, and the real ports are the
 * three thin lines at the bottom of `NowPlayingPorts.kt`.
 */
class FakePlayerPort(
    var snapshot: PlayerSnapshot = snapshot(),
    /** The queue as a count: the 4 000-row card the acceptance names. */
    var queueRows: List<TrackRow> = queueOf(4),
) : PlayerPort {
    val commands = mutableListOf<PlayerCommand>()
    val settings = mutableListOf<AudioSettings>()
    val sleeps = mutableListOf<SleepTimer?>()
    var positionReads = 0
        private set
    var position: PositionInfo = PositionInfo(0.0, 180.0, 180.0, true)
    var currentAudio: AudioSettings = FakePlayerPort.audio()

    override fun snapshot(): PlayerSnapshot = snapshot

    override fun position(): PositionInfo {
        positionReads += 1
        return position
    }

    override fun command(cmd: PlayerCommand, tapNs: Long) {
        commands += cmd
    }

    override fun queueSource(): NzRowSource = RowSource(queueRows)

    override fun audio(): AudioSettings = currentAudio

    override fun setAudio(settings: AudioSettings) {
        currentAudio = settings
        this.settings += settings
    }

    override fun setSleep(timer: SleepTimer?) {
        sleeps += timer
        snapshot = snapshot.copy(sleep = timer?.let { sleepStateFor(it) })
    }

    private fun sleepStateFor(timer: SleepTimer) = when (timer) {
        is SleepTimer.Minutes -> uniffi.nz_ffi.SleepState(timer.minutes, false)
        is SleepTimer.EndOfTrack -> uniffi.nz_ffi.SleepState(0u, true)
    }

    companion object {
        fun audio(quality: Quality = Quality.AUTO, speed: Float = 1f) = AudioSettings(
            quality = quality,
            crossfadeS = 0f,
            trimSilence = false,
            normalization = uniffi.nz_ffi.Normalization.OFF,
            eqDb = List(10) { 0f },
            eqPreset = null,
            bassDb = 0f,
            mono = false,
            balance = 0f,
            speed = speed,
            meteredCap = Quality.AUTO,
        )

        /** A queue of [size] rows, titled « Titre n », as a play queue builds it. */
        fun queueOf(size: Int): List<TrackRow> = (0 until size).map { index ->
            TrackRow(
                key = "${9001 + index}",
                id = "${9001 + index}",
                title = "Titre $index",
                artists = "Artiste ${index % 7}",
                album = "Album ${index % 11}",
                albumId = "5${index % 11}",
                durationS = (180 + index % 60).toUInt(),
                cover = null,
                explicit = false,
                liked = false,
                disliked = false,
                unavailable = false,
                downloaded = false,
                source = "deezer",
                position = index.toUInt(),
            )
        }

        fun track(
            key: String = "9001",
            title: String = "Le chant des sirènes",
            episode: Boolean = false,
            albumId: String? = "501",
        ) = NowPlaying(
            key = key,
            id = key,
            title = title,
            artists = "Fauve",
            album = "Vieux frères",
            albumId = albumId,
            artistId = "42",
            cover = "/api/cover/501",
            durationS = 215.0,
            isEpisode = episode,
            quality = "320",
            source = "deezer",
            liked = false,
        )

        fun snapshot(
            state: PlayState = PlayState.PLAYING,
            current: NowPlaying? = track(),
            index: UInt? = 1u,
            len: UInt = 4u,
            shuffle: Boolean = false,
            repeat: Repeat = Repeat.OFF,
        ) = PlayerSnapshot(
            state = state,
            current = current,
            index = index,
            len = len,
            shuffle = shuffle,
            repeat = repeat,
            volume = 1f,
            muted = false,
            streaming = true,
            sleep = null,
            error = null,
            fault = null,
            version = 1uL,
        )
    }
}

/** The core's screens, counting: what each screen asks for is the acceptance. */
class FakeService(
    val cachedLyrics: LyricsData? = null,
    val fresh: LyricsData = lyrics(),
    val similar: List<TrackRow> = similarRows(),
    val credits: TrackCreditsData = credits(),
    val coverPath: String? = null,
    val speeds: MutableMap<String, Float> = mutableMapOf(),
    val speedChoices: List<Float> = listOf(0.75f, 1f, 1.25f, 1.5f, 1.75f, 2f, 2.5f, 3f),
    var failLyrics: String? = null,
    var failSimilar: Boolean = false,
) : NowPlayingService {
    var lyricCalls = 0
        private set
    var cachedCalls = 0
        private set
    var similarCalls = 0
        private set
    var creditsCalls = 0
        private set
    var coversAsked = mutableListOf<Pair<String, Int>>()
    val sorted = mutableListOf<Float>()

    override suspend fun cachedLyrics(key: String): LyricsData? {
        cachedCalls += 1
        return cachedLyrics
    }

    override suspend fun lyrics(key: String): LyricsData {
        lyricCalls += 1
        failLyrics?.let { throw IllegalStateException(it) }
        return fresh
    }

    override suspend fun similarTracks(key: String): NzRowSource? {
        similarCalls += 1
        if (failSimilar) return null
        return object : NzRowSource {
            override fun len(): Int = similar.size

            override fun window(start: Int, count: Int): List<TrackRow> =
                similar.subList(start.coerceAtLeast(0), minOf(start + count, similar.size))
        }
    }

    override fun cardLines(lyrics: LyricsData): List<CardLineData> {
        // The rules are Rust's; the fake repeats the two the tests exercise: a
        // marker is not a line, and `source` keeps the original index.
        val out = mutableListOf<CardLineData>()
        lyrics.synced.forEachIndexed { index, line ->
            val text = line.text.trim()
            if (text.isEmpty() || text.none { it.isLetterOrDigit() }) return@forEachIndexed
            out += CardLineData(text = text, timeMs = line.timeMs, source = index.toUInt())
        }
        return out
    }

    override fun cardDefaultLine(lyrics: LyricsData, activeSource: Long): Int {
        val lines = cardLines(lyrics)
        if (lines.isEmpty()) return -1
        val exact = lines.indexOfFirst { it.source.toLong() == activeSource }
        if (exact >= 0) return exact
        var before = 0
        lines.forEachIndexed { index, line ->
            if (line.source.toLong() < activeSource) before = index
        }
        return before
    }

    override fun cardToggle(selected: List<Int>, index: Int): CardSelection {
        val current = selected.sorted().distinct()
        if (current.contains(index)) {
            if (current.size <= 1) return CardSelection(current, "min")
            return CardSelection(current.filter { it != index }, null)
        }
        if (current.size >= 3) return CardSelection(current, "max")
        return CardSelection((current + index).sorted(), null)
    }

    override fun cardTexts(lyrics: LyricsData, selected: List<Int>): List<String> {
        val lines = cardLines(lyrics)
        return selected.sorted().distinct().filter { it in lines.indices }.take(3)
            .map { lines[it].text }
    }

    override fun cardFilename(artist: String, title: String): String =
        "neezer-${artist.lowercase().replace(' ', '-')}-${title.lowercase().replace(' ', '-')}.png"

    override fun speedChoices(): List<Float> = speedChoices

    override fun speed(show: String): Float = speeds[show] ?: 1f

    override fun setSpeed(show: String, rate: Float) {
        if (rate == 1f) speeds.remove(show) else speeds[show] = rate
    }

    override suspend fun coverPath(url: String, sizePx: Int): String? {
        coversAsked += url to sizePx
        return coverPath
    }

    override suspend fun cachedCredits(key: String): TrackCreditsData? = null

    override suspend fun credits(key: String): TrackCreditsData {
        creditsCalls += 1
        return credits
    }

    companion object {
        fun lyrics(): LyricsData = LyricsData(
            synced = listOf(
                LyricLine(timeMs = 0u, text = "Il est cinq heures"),
                LyricLine(timeMs = 12_000u, text = "♪"),
                LyricLine(timeMs = 24_000u, text = "Paris s'éveille"),
                LyricLine(timeMs = 36_000u, text = "Je me réveille"),
            ),
            plain = "Il est cinq heures\n♪\nParis s'éveille",
            source = "archive",
            empty = false,
            bytes = 214uL,
        )

        fun emptyLyrics(): LyricsData = LyricsData(
            synced = emptyList(),
            plain = "",
            source = "",
            empty = true,
            bytes = 8uL,
        )

        fun row(key: String, title: String) = TrackRow(
            key = key,
            id = key,
            title = title,
            artists = "Fauve",
            album = "Vieux frères",
            albumId = "501",
            durationS = 200u,
            cover = null,
            explicit = false,
            liked = false,
            disliked = false,
            unavailable = false,
            downloaded = false,
            source = "deezer",
            position = 0u,
        )

        fun similarRows(): List<TrackRow> = listOf(
            // The server puts the SEED first (`radio.build`); the panel filters it.
            row("9001", "Le chant des sirènes"),
            row("9002", "Blizzard"),
            row("9003", "Kané"),
        )

        fun credits(): TrackCreditsData = TrackCreditsData(
            credits = listOf(
                uniffi.nz_ffi.CreditGroupData(
                    role = "composer",
                    label = "Compositeur",
                    people = listOf(
                        uniffi.nz_ffi.CreditPersonData(
                            name = "Nina Simone",
                            artistId = "dddd",
                            credited = 3u,
                        ),
                    ),
                ),
            ),
            info = CreditsInfoData(
                isrc = "FR123",
                released = "2001-03-12",
                label = "Blue Note",
                copyright = "(c) 2001",
                album = "Vieux frères",
            ),
            empty = false,
        )
    }
}

/** A `NzRowSource` over a fixed list: the window's pages, as a real handle answers. */
class RowSource(private val rows: List<TrackRow>) : NzRowSource {
    var windows = 0
        private set

    override fun len(): Int = rows.size

    override fun window(start: Int, count: Int): List<TrackRow> {
        windows += 1
        val from = start.coerceIn(0, rows.size)
        val to = (from + count).coerceIn(from, rows.size)
        return rows.subList(from, to)
    }
}
