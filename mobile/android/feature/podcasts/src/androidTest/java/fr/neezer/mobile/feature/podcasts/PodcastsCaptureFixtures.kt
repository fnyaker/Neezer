package fr.neezer.mobile.feature.podcasts

import uniffi.nz_ffi.EpisodeData
import uniffi.nz_ffi.EpisodeMarkerData
import uniffi.nz_ffi.EpisodePositionData
import uniffi.nz_ffi.ShowData
import uniffi.nz_ffi.TrackRow
import uniffi.nz_ffi.TranscribeSettingsData
import uniffi.nz_ffi.TranscriptInfoData

/**
 * The capture's fixtures: the same shapes the device paints, built here because
 * `src/test`'s own fixtures are not on the instrumented test's classpath (a
 * Gradle source set is a classpath, and this one holds no JVM fake).
 *
 * Nothing here is a rule: every number matches `PodcastsFakes.kt`'s, so the
 * screenshot on the emulator and the one Robolectric renders show the SAME
 * screen — a difference between them would be a fixture, not a bug.
 */
internal fun show(
    id: String,
    title: String,
    deezer: String? = null,
    episodes: Int = 0,
    autoArchive: Boolean = false,
    autoTranscribe: Boolean = false,
    language: String = "auto",
): ShowData = ShowData(
    id = id,
    deezerId = deezer,
    title = title,
    description = "Une heure de récits",
    cover = null,
    subscribed = true,
    local = false,
    status = "ok",
    episodeCount = episodes.toUInt(),
    archivedCount = 2u,
    autoArchive = autoArchive,
    autoTranscribe = autoTranscribe,
    transcribeLanguage = language,
    transcribedCount = null,
    archiveLine = "2 sur $episodes épisodes archivés sur le serveur",
    subscribeUrl = deezer?.let { "https://www.deezer.com/show/$it" },
)

internal fun episode(
    id: String,
    title: String = "Épisode $id",
    archived: Boolean = true,
    chapters: Int = 0,
    published: Long = 1_794_960_000,
    transcript: TranscriptInfoData? = null,
    position: EpisodePositionData? = null,
    markers: List<EpisodeMarkerData> = emptyList(),
): EpisodeData = EpisodeData(
    id = id,
    deezerId = id,
    title = title,
    description = "Un épisode de plus.",
    durationS = 3_600u,
    durationLabel = "1:00:00",
    published = published,
    publishedLabel = "18 nov. 2026",
    archived = archived,
    unavailable = false,
    explicit = false,
    chapterCount = chapters.toUInt(),
    chaptersLabel = if (chapters > 1) "$chapters chapitres" else "$chapters chapitre",
    status = if (archived) "completed" else "",
    cover = null,
    channelId = "show-1",
    channelTitle = "Les Histoires",
    transcript = transcript,
    position = position,
    markers = markers,
    about = true,
    row = TrackRow(
        key = id,
        id = id,
        title = title,
        artists = "Les Histoires",
        album = "Les Histoires",
        albumId = "show-1",
        durationS = 3_600u,
        cover = null,
        explicit = false,
        liked = false,
        disliked = false,
        unavailable = false,
        downloaded = archived,
        source = "podcast",
        position = 0u,
    ),
)

internal fun marker(id: String, episodeId: String, positionMs: Int, label: String = ""): EpisodeMarkerData =
    EpisodeMarkerData(
        id = id,
        episodeId = episodeId,
        positionMs = positionMs.toUInt(),
        seconds = (positionMs / 1_000).toUInt(),
        timeLabel = PodcastsCopy.duration(positionMs / 1_000),
        label = label,
    )

internal fun resumeAt(seconds: Int) = EpisodePositionData(
    seconds = seconds.toUInt(),
    durationS = 3_600u,
    finished = false,
    fraction = seconds / 3_600.0,
    remainingS = (3_600 - seconds).toUInt(),
    pending = false,
)

internal fun transcriptInfo(status: String, progress: Double = 0.0): TranscriptInfoData = TranscriptInfoData(
    status = status,
    progress = progress,
    percent = (progress * 100).toInt().toUInt(),
    language = "fr",
    source = "server",
    segmentCount = 240u,
    errorCode = null,
    active = status == "queued" || status == "running" || status == "leased",
    ready = status == "ready",
)

internal fun settings(available: Boolean = true, reason: String? = null): TranscribeSettingsData =
    TranscribeSettingsData(
        available = available,
        enabled = true,
        reason = reason,
        language = "fr",
        languages = TRANSCRIBE_LANGUAGES,
        dailyMinutes = 120u,
        maxMinutes = 240u,
        queued = 1u,
        running = 1u,
        ready = 8u,
        error = 0u,
        engine = "whisper.cpp",
        model = "ggml-small.bin",
    )
