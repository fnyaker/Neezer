# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Fetch-and-archive a Deezer track on demand, plus helpers to import metadata.

``ensure_archived`` is the streaming seam: when a Deezer-backed Track has no
file yet, it resolves a playable URL, stream-decrypts the (preferably FLAC)
audio into the track's archive path, tags it, and updates the row. Once on
disk the track behaves like any other local file (transcoding, caching, cover
extraction all work unchanged).
"""

from __future__ import annotations

import logging
import os
import os.path
import re
import shutil
import threading
import time
from pathlib import Path
from uuid import uuid4

from ..db import Album, Artist, Playlist, Track, PodcastChannel, PodcastEpisode
from ..db import db as _database
from . import ids, library
from .metadata import meta_from_gw, tag_file
from .provider import EXT_FOR_FORMAT, NOMINAL_BITRATE, DeezerError

logger = logging.getLogger(__name__)

# Audio mimetype for an archived file, by extension (FLAC normally; MP3 when
# Deezer had no lossless source for the track).
_MIME_FOR_EXT = {".flac": "audio/flac", ".mp3": "audio/mpeg"}


def _mime_for_path(path) -> str:
    return _MIME_FOR_EXT.get(os.path.splitext(str(path))[1].lower(), "audio/flac")


def _fixed_ext_path(path: str, fmt: str) -> str:
    """The archive path's extension was guessed at import time; correct it to
    match the format actually obtained (e.g. FLAC unavailable -> MP3)."""
    ext = EXT_FOR_FORMAT.get(fmt, ".mp3")
    base, cur_ext = os.path.splitext(path)
    return base + ext if cur_ext.lower() != ext else path

_LINK_RE = re.compile(
    r"(?:deezer\.com|deezer\.page\.link|dzr\.page\.link)/(?:[a-z]{2}/)?"
    r"(track|album|playlist|artist|show|episode)/(\d+)"
)


def parse_deezer_ref(ref: str):
    """Parse a Deezer URL or ``type:id`` / ``type id`` into ``(type, id)``."""
    ref = str(ref).strip()
    m = _LINK_RE.search(ref)
    if m:
        return m.group(1), m.group(2)
    m = re.fullmatch(r"(track|album|playlist|artist|show|episode)[ :/](\d+)", ref)
    if m:
        return m.group(1), m.group(2)
    if ref.isdigit():
        return "track", ref
    raise ValueError(f"unrecognized Deezer reference: {ref!r}")


def _bitrate_for(path: str, fmt: str, duration: int) -> int:
    try:
        size = os.path.getsize(path)
        if duration > 0:
            return max(1, int(size * 8 / duration / 1000))
    except OSError:
        pass
    return NOMINAL_BITRATE.get(fmt, 320)


def ensure_archived(provider, track: Track) -> None:
    """Fetch + decrypt + tag the FLAC for `track` into its archive path.

    Idempotent and serialized per Deezer track id, so concurrent plays and the
    preloader never download the same track twice.
    """
    if not track.deezer_id:
        return
    if os.path.isfile(track.path):
        return

    with provider.track_lock(track.deezer_id):
        if os.path.isfile(track.path):
            return

        url, fmt, info, used_id = _resolve_or_flag(provider, track)
        track.path = _fixed_ext_path(track.path, fmt)

        logger.info("Archiving Deezer track %s (%s) -> %s", track.deezer_id, fmt, track.path)
        # Downloaded and tagged under a name nobody serves, THEN published: see
        # _tag_archive for why the tags may never be written on the real path.
        dest = Path(track.path)
        staged = dest.with_name(f"{dest.name}.{uuid4().hex}.stage.part")
        dest.parent.mkdir(parents=True, exist_ok=True)
        try:
            provider.download_to(url, used_id, staged)
            cover = _tag_archive(provider, staged, info, dest.suffix)
            os.replace(staged, dest)
        finally:
            _safe_unlink(staged)
        _finalize_archive(provider, track, fmt, info, cover)


def _resolve_or_flag(provider, track: Track):
    """``provider.resolve`` that records the verdict on the way through.

    EVERY path that fetches audio goes through here — a live stream, the
    background archiver, the download queue, the CLI — so a track Deezer has
    dropped is flagged whoever noticed, not only when someone happened to press
    play on it. Conversely a resolve that succeeds clears a stale verdict.

    Only ``TrackUnavailable`` counts: it is the one error that means "Deezer
    answered, and the answer is that there is nothing to play". Network and
    session failures pass through untouched.
    """
    from .provider import TrackUnavailable

    try:
        resolved = provider.resolve(track.deezer_id)
    except TrackUnavailable:
        logger.info("Deezer has no source for track %s; flagging it", track.deezer_id)
        library.mark_unavailable(track)
        raise
    library.clear_unavailable(track)
    return resolved


def _tag_archive(provider, path, info: dict, suffix: str) -> bytes | None:
    """Tag a downloaded file that is NOT yet at its archive path; returns the cover.

    Tagging rewrites the file in place (mutagen moves the audio along to make
    room for a 1000 px cover), and once a file is at its archive path it is
    being served: ``/api/stream`` and Subsonic's stream send it, the silence
    measurement reads it (and caches its answer for good). Tagged after the
    rename, any of them reading in that window got a file half moved. So the
    file is tagged under its staging name and published complete.
    """
    meta = meta_from_gw(info)
    cover = provider.fetch_cover(meta.get("md5_image"))
    try:
        tag_file(Path(path), meta, cover, suffix)
    except Exception as exc:  # tagging must never break playback
        logger.warning("Tagging failed for %s: %s", path, exc)
    return cover


def _finalize_archive(provider, track: Track, fmt: str, info: dict, cover=None) -> None:
    """Tag the freshly-archived file, and archive everything ELSE about it.

    Archiving the audio is only half the promise. A track is "unavailable" when
    neither Deezer nor the disk has it — so once the audio is on disk, nothing
    about that track may still depend on Deezer being reachable: the tags, the
    cover, the lyrics, the loudness, and the database row all get filled in here,
    from the authoritative ``song.getData`` payload.

    That payload is richer than whatever dict the row was first created from (a
    playlist or album listing carries a subset), so the row is REFRESHED from it
    rather than only having its bitrate stamped.
    """
    # `cover` is what _tag_archive fetched for the tags, before publication.
    if cover:
        # Archive the art on disk too (a cover.jpg sidecar) so it is served
        # locally, exactly like the audio, even if Deezer later drops the album.
        try:
            library.save_album_cover(track.folder, cover)
        except Exception as exc:
            logger.warning("Saving cover sidecar failed for %s: %s", track.path, exc)

    # Refresh the row from the authoritative payload: title/version, credited
    # artists, disc & track number, release date, duration. The row may have
    # been created from a playlist listing, which carries a thinner subset — and
    # once Deezer drops the track, whatever is in the row IS the metadata.
    library.refresh_track_metadata(track, info)

    track.bitrate = _bitrate_for(track.path, fmt, track.duration)
    track.has_art = bool(cover)
    track.last_modification = int(time.time())
    # The audio is now ON DISK, which is the whole point of archiving: from here
    # on this track plays without Deezer, forever, whatever Deezer does with it
    # afterwards. So any earlier "unavailable" verdict is void by construction.
    track.unavailable = None
    track.save()

    # Measure what the whole track sounds like — its tempo and its style — so
    # the player's animation engine is handed them instead of spending the first
    # fifteen seconds of every play working them out. Queued on its own daemon
    # thread: it has no deadline and must never be why an archive waits.
    try:
        from . import analysis as analysis_mod

        analysis_mod.queue_analysis(track, provider)
    except Exception as exc:  # analysis is never a condition for archiving
        logger.warning("Queueing analysis failed for %s: %s", track.path, exc)

    # And a metadata sidecar, so the archive describes itself: the tags cannot
    # hold Deezer ids or contributor roles, and a database restored from an old
    # backup would otherwise have no way back to them.
    try:
        library.save_track_metadata(track, info)
        library.update_track_facts(track, library.deezer_facts(track, info), "deezer")
    except Exception as exc:
        logger.debug("Saving the metadata sidecar failed for %s: %s", track.path, exc)
    # And who wrote, composed and produced it, in the database: the credits
    # panel and each person's credits page read them there.
    if "SNG_CONTRIBUTORS" in info:
        try:
            from .. import trackcredits

            trackcredits.store(track.id, info["SNG_CONTRIBUTORS"], "deezer")
        except Exception:
            logger.debug("Storing the credits failed for %s", track.path, exc_info=True)

    # MusicBrainz completes what Deezer does not state, later and politely: a
    # queue entry here, never a wait.
    try:
        from ..enrich import queue as enrich_queue

        enrich_queue.queue_track(track.id)
    except Exception:
        logger.debug("Queueing enrichment failed for %s", track.path, exc_info=True)

    # Archive lyrics alongside the audio (a .lrc sidecar + embedded plain text).
    # On the archive hot path we only consult Deezer (already logged in for the
    # audio) — the public LRCLIB gap-fill happens lazily on first view / via the
    # `deezer lyrics` backfill, so bulk archiving never fans out to a second
    # service. Best-effort: a lyrics miss must never break the archive.
    try:
        from . import lyrics as lyrics_mod

        lyrics_mod.ensure_lyrics(provider, track, allow_lrclib=False)
    except Exception as exc:
        logger.debug("Archiving lyrics failed for %s: %s", track.path, exc)


def ensure_episode_archived(provider, episode: PodcastEpisode) -> None:
    """Fetch a podcast episode's MP3 into its archive path (idempotent).

    Same contract as ``ensure_archived`` but for spoken-word episodes: the
    source is an external MP3 (no FLAC, no Blowfish), archived once then served
    locally. Serialized per Deezer episode id.
    """
    if episode.path and os.path.isfile(episode.path):
        return

    with provider.track_lock(f"ep:{episode.deezer_id}"):
        if episode.path and os.path.isfile(episode.path):
            return

        path = library.episode_archive_path(
            provider.archive_dir,
            episode.channel.title,
            episode.publish_date,
            episode.title,
        )
        episode.status = "downloading"
        episode.save()
        logger.info("Archiving Deezer episode %s -> %s", episode.deezer_id, path)
        try:
            url = provider.resolve_episode(episode)
            provider.download_episode_to(url, path)
        except Exception:
            episode.status = "error"
            episode.save()
            raise

        _finalize_episode(episode, path, provider)


def open_live_episode_stream(provider, episode: PodcastEpisode, on_abort=None):
    """Stream-first playback for a not-yet-archived podcast episode.

    The episode counterpart of ``open_live_stream``, and for podcasts it is not
    an optimisation but the difference between working and not: an hour-long
    show is tens of megabytes, and archiving it in full before sending the first
    byte meant a long episode simply never started. Resolving the URL is a row
    lookup, so this returns almost immediately and the listener hears audio as
    it arrives, while every chunk is teed to the archive.

    Returns ``(mimetype, generator)``. Like the track version, the first live
    play is not seekable (no Content-Length) and a client that disconnects early
    drops the partial and falls back to ``on_abort`` for background archiving.
    """
    url = provider.resolve_episode(episode)
    path = library.episode_archive_path(
        provider.archive_dir,
        episode.channel.title,
        episode.publish_date,
        episode.title,
    )
    dest = Path(path)
    mimetype = episode.mimetype or "audio/mpeg"

    def generate():
        # Archived in the gap between resolve and first read? Serve from disk.
        if dest.is_file():
            with open(dest, "rb") as fh:
                yield from iter(lambda: fh.read(65536), b"")
            return

        tmp = dest.with_name(f"{dest.name}.{uuid4().hex}.part")
        complete = False
        try:
            tmp.parent.mkdir(parents=True, exist_ok=True)
            with open(tmp, "wb") as fh:
                for chunk in provider.iter_episode(url):
                    fh.write(chunk)
                    yield chunk
            complete = True
        finally:
            try:
                if complete and not dest.is_file():
                    with provider.track_lock(f"ep:{episode.deezer_id}"):
                        if dest.is_file():
                            _safe_unlink(tmp)
                        else:
                            os.replace(tmp, dest)
                            _finalize_episode(episode, str(dest))
                else:
                    _safe_unlink(tmp)
                    if not complete and not dest.is_file() and on_abort is not None:
                        on_abort()
            except Exception:
                logger.warning(
                    "Live episode archive finalize failed for %s",
                    episode.deezer_id,
                    exc_info=True,
                )

    return mimetype, generate()


def _finalize_episode(episode: PodcastEpisode, path: str, provider=None) -> None:
    """Publish an episode's archive on the row (shared by both fetch paths)."""
    episode.path = path
    episode.bitrate = _bitrate_for(path, "MP3_128", episode.duration)
    episode.status = "completed"
    episode.save()
    # A file read, off the play path: the chapters the episode itself carries.
    try:
        from . import chapters

        chapters.ensure(episode)
    except Exception:
        logger.debug("Could not read the chapters of episode %s", episode.deezer_id, exc_info=True)
    # Archive the show's art alongside its audio. Once the episodes are local the
    # picture has to be too, or a show that leaves Deezer keeps its episodes and
    # loses its face. Best-effort, once per show.
    if provider is not None:
        try:
            channel = episode.channel
            if not library.show_cover_file(provider.archive_dir, channel):
                md5 = channel.cover_art_md5 or episode.image_md5
                if md5:
                    art = provider.fetch_cover(md5)
                    if art:
                        library.save_show_cover(provider.archive_dir, channel, art)
        except Exception as exc:
            logger.debug("Could not archive show art: %s", exc)
    # A new episode of a show marked auto_transcribe joins the queue. This is a
    # row write and a wake-up, never the engine: a transcription error must not
    # fail the archive, and archiving must not wait for one.
    try:
        from ..transcribe import auto_enqueue

        auto_enqueue(episode)
    except Exception:
        logger.debug(
            "Could not queue the transcript of episode %s",
            episode.deezer_id,
            exc_info=True,
        )


def find_local_episode(episode_id) -> PodcastEpisode | None:
    """The already-imported PodcastEpisode for a Deezer episode id, or None."""
    try:
        return PodcastEpisode[ids.episode_uuid(episode_id)]
    except PodcastEpisode.DoesNotExist:
        return None


def import_show(provider, user, show_id, episode_limit=None) -> PodcastChannel:
    """Import a Deezer show + its episodes (metadata only) as rows.

    ``episode_limit`` caps how many recent episodes to import (None = all).
    """
    page = provider.get_show_page(show_id, nb=episode_limit or 200)
    channel = library.upsert_channel(user, library.normalize_show(page["DATA"]))
    if episode_limit:
        raw_eps = page.get("EPISODES", {}).get("data", [])[:episode_limit]
    else:
        raw_eps = provider.get_show_episodes(show_id)
    for raw in raw_eps:
        library.upsert_episode(channel, library.normalize_episode(raw))
    return channel


LIVE_GRACE_SECONDS = 10.0

LIVE_REUSE_SECONDS = 1.5

LIVE_ORPHAN_PACE = 0.4

LIVE_PUBLISH_WAIT = 5.0

LIVE_STALL_SECONDS = 130.0

_LIVE_CHUNK = 65536

_live_lock = threading.Lock()

_live_downloads: dict = {}


def _live_key(provider, track) -> tuple:
    """One live download per (archive, track): a seek reload joins the running one."""
    return (
        str(getattr(provider, "archive_dir", None) or ""),
        str(getattr(track, "deezer_id", None) or track.path),
    )


def _live_lookup(key):
    with _live_lock:
        live = _live_downloads.get(key)
    if live is None:
        return None
    if live.stopping or (live.finished and not live.published):
        with _live_lock:
            if _live_downloads.get(key) is live:
                del _live_downloads[key]
        return None
    return live


def _live_register(key, live):
    """``live``, or the download that was already running for that key."""
    with _live_lock:
        running = _live_downloads.get(key)
        if running is not None and (running.running or running.published):
            return running
        _live_downloads[key] = live
        return live


def _live_release(key, live) -> None:
    with _live_lock:
        if _live_downloads.get(key) is live:
            del _live_downloads[key]


def _read_file(path):
    with open(path, "rb") as fh:
        yield from iter(lambda: fh.read(_LIVE_CHUNK), b"")


class _LiveArchive:
    """One track's live download, independent of who is reading it.

    ``open_live_stream`` used to write the fetched bytes to disk and yield them
    from the same generator: the yield blocks on the client's socket, so a
    browser that had buffered enough stopped reading and the download stopped
    with it — the archive stayed a ``.part`` and ``/ready`` said ``streaming``
    for as long as the tab held the response. A seek then waited for a download
    paced by playback, sometimes minutes.

    Here the producer is a thread of its own. It writes the decrypted bytes to
    a ``.part`` at link speed whatever the listener does, then tags a COPY and
    renames it onto the archive path (readers keep reading the raw file and
    never see a half-tagged one), and finalises the row. Readers tail that
    ``.part``, each from the byte it needs. When the last reader leaves the
    download keeps going for ``LIVE_GRACE_SECONDS`` so a seek reload of the same
    track joins it instead of downloading it again; after the grace it stops and
    hands what is left to ``on_abort`` (the background queue, at PREFETCH).
    """

    def __init__(self, provider, track, url, fmt, info, used_id, on_abort):
        self.provider = provider
        self.track = track
        self.url = url
        self.fmt = fmt
        self.info = info
        self.used_id = used_id
        self.on_abort = on_abort
        self.dest = Path(track.path)
        self.mimetype = _mime_for_path(self.dest)
        self.tmp = self.dest.with_name(f"{self.dest.name}.{uuid4().hex}.part")
        self.published = False
        self._key = _live_key(provider, track)
        self._cond = threading.Condition()
        self._written = 0
        self._downloaded = False
        self._done = False
        self._error = None
        self._started = False
        self._stopping = False
        self._thread = None
        self._readers = 0
        self._abandoned_at = None
        self._decayed = False
        self._abort_called = False
        self._tmp_unlinked = False

    @property
    def running(self) -> bool:
        with self._cond:
            return self._started and not self._done and not self._stopping

    @property
    def stopping(self) -> bool:
        with self._cond:
            return self._stopping

    @property
    def finished(self) -> bool:
        with self._cond:
            return self._done

    def listen(self):
        """A response body: what this track sounds like, from its first byte."""
        return self._read()

    def _read(self):
        self._enter()
        try:
            if self.dest.is_file():
                yield from _read_file(self.dest)
                return
            self._start()
            with self._cond:
                downloading = not self._downloaded and not self._done
            if not downloading:
                self._wait_published(LIVE_PUBLISH_WAIT)
                if self.dest.is_file():
                    yield from _read_file(self.dest)
                    return
                with self._cond:
                    done = self._done
                    error = self._error
                if done:
                    raise error or DeezerError(
                        f"the live stream of {self.track.deezer_id} ended"
                    )
            try:
                fh = open(self.tmp, "rb")
            except FileNotFoundError:
                if self.dest.is_file():
                    yield from _read_file(self.dest)
                    return
                raise DeezerError(
                    f"the live stream of {self.track.deezer_id} is gone"
                ) from None
            try:
                seen = 0
                idle_since = time.monotonic()
                while True:
                    data = fh.read1(_LIVE_CHUNK)
                    if data:
                        seen += len(data)
                        idle_since = time.monotonic()
                        yield data
                        continue
                    with self._cond:
                        done = self._done
                        written = self._written
                    if done and written <= seen:
                        break
                    if time.monotonic() - idle_since > LIVE_STALL_SECONDS:
                        raise DeezerError(
                            f"the live stream of {self.track.deezer_id} stalled"
                        )
                    with self._cond:
                        if not self._done:
                            self._cond.wait(0.5)
                if self._error is not None:
                    raise self._error
            finally:
                fh.close()
        finally:
            self._leave()

    def _enter(self):
        with self._cond:
            self._readers += 1
            self._abandoned_at = None

    def _leave(self):
        with self._cond:
            self._readers -= 1
            idle = self._readers <= 0
            done = self._done
            if idle and not done:
                self._abandoned_at = time.monotonic()
        if idle and done:
            self._unlink_tmp()

    def _start(self):
        with self._cond:
            if self._started:
                return
            self._started = True
        self.dest.parent.mkdir(parents=True, exist_ok=True)
        open(self.tmp, "wb").close()
        self._thread = threading.Thread(
            target=self._produce, name="live-archive", daemon=True
        )
        self._thread.start()

    def _stop_wanted(self) -> bool:
        with self._cond:
            abandoned = self._abandoned_at
        if abandoned is None:
            return False
        return (time.monotonic() - abandoned) > LIVE_GRACE_SECONDS

    def _orphan_pace(self) -> float:
        """How long to wait between chunks once nobody is listening (0: full speed).

        The last listener leaving is usually a SKIP, and the next track must get
        the link — but a seek reload of the same track can land a moment later
        (the player asks at 600 ms, then every second). So an abandoned download
        keeps full speed for ``LIVE_REUSE_SECONDS`` (enough for a reload to join
        and clear it), then hands back all but ~160 KB/s until the grace ends.
        """
        with self._cond:
            abandoned = self._abandoned_at
        if abandoned is None:
            return 0.0
        if (time.monotonic() - abandoned) <= LIVE_REUSE_SECONDS:
            return 0.0
        self._decay()
        return LIVE_ORPHAN_PACE

    def _wait_published(self, timeout: float) -> None:
        """Wait (bounded) for a download that is finished to be published."""
        deadline = time.monotonic() + timeout
        with self._cond:
            while not (self.published or self._done):
                left = deadline - time.monotonic()
                if left <= 0:
                    return
                self._cond.wait(min(left, 0.25))

    def _decay(self):
        if self._decayed:
            return
        self._decayed = True
        try:
            from . import workload

            workload.renice()
        except Exception:
            logger.debug("Could not deprioritise the orphan download", exc_info=True)

    def _produce(self):
        try:
            _database.connect(reuse_if_open=True)
        except Exception:
            pass
        stopped = False
        try:
            self.dest.parent.mkdir(parents=True, exist_ok=True)
            stream = self.provider.iter_decrypted(self.url, self.used_id)
            closer = getattr(stream, "close", None)
            try:
                with open(self.tmp, "wb") as fh:
                    for chunk in stream:
                        fh.write(chunk)
                        fh.flush()
                        with self._cond:
                            self._written += len(chunk)
                            self._cond.notify_all()
                        if self._stop_wanted():
                            with self._cond:
                                self._stopping = True
                            stopped = True
                            self._decay()
                            break
                        pace = self._orphan_pace()
                        if pace:
                            time.sleep(pace)
            finally:
                if closer is not None:
                    closer()
            with self._cond:
                self._downloaded = not stopped
                self._cond.notify_all()
            if not stopped:
                self._publish()
        except Exception as exc:
            with self._cond:
                self._error = exc
            logger.warning("Live download of %s failed: %s", self.track.deezer_id, exc)
        finally:
            with self._cond:
                idle = self._readers <= 0
                published = self.published
            if not published and not self.dest.is_file():
                self._abort()
            with self._cond:
                self._done = True
                self._cond.notify_all()
            _live_release(self._key, self)
            try:
                _database.close()
            except Exception:
                pass
            if idle:
                self._unlink_tmp()

    def _abort(self):
        if self._abort_called or self.on_abort is None:
            return
        self._abort_called = True
        try:
            self.on_abort()
        except Exception:
            logger.debug("Live archive on_abort failed", exc_info=True)

    def _publish(self):
        stage = self.dest.with_name(f"{self.dest.name}.{uuid4().hex}.stage.part")
        cover = None
        try:
            with self.provider.track_lock(self.track.deezer_id):
                if self.dest.is_file():
                    return
                shutil.copyfile(self.tmp, stage)
                cover = _tag_archive(self.provider, stage, self.info, self.dest.suffix)
                os.replace(stage, self.dest)
                with self._cond:
                    self.published = True
                    self._cond.notify_all()
                _finalize_archive(self.provider, self.track, self.fmt, self.info, cover)
        except Exception:
            logger.warning(
                "Live archive finalize failed for %s",
                self.track.deezer_id,
                exc_info=True,
            )
        finally:
            _safe_unlink(stage)

    def _unlink_tmp(self):
        with self._cond:
            if self._tmp_unlinked:
                return
            self._tmp_unlinked = True
        _safe_unlink(self.tmp)


def open_live_stream(provider, track: Track, on_abort=None):
    """Stream-first playback for a cold (not-yet-archived) track.

    Resolves the source synchronously (fast — just URL/token calls), then returns
    ``(mimetype, generator)``. The download itself runs on its own thread and is
    shared by every listener of that track: the generator tails the growing
    ``.part`` and never paces the fetch, so the archive completes in download
    time whatever the client does (see ``_LiveArchive``). On completion the file
    is tagged and published atomically, so the very next play is served from
    disk — seekable, transcodable.

    The generator still hands its first block to the caller's first ``next()``,
    so a source that refuses (no stream, not audio, empty) is that route's 502
    rather than a response dying half-sent. A listener that leaves does not stop
    the download: it runs ``LIVE_GRACE_SECONDS`` more so a seek reload of the
    same track reuses it, and only then stops and calls ``on_abort`` (the
    background queue takes the rest).
    """
    key = _live_key(provider, track)
    existing = _live_lookup(key)
    if existing is not None:
        return existing.mimetype, existing.listen()

    if os.path.isfile(track.path):
        return _mime_for_path(track.path), _read_file(track.path)

    url, fmt, info, used_id = _resolve_or_flag(provider, track)
    track.path = _fixed_ext_path(track.path, fmt)
    live = _LiveArchive(provider, track, url, fmt, info, used_id, on_abort)
    winner = _live_register(key, live)
    if winner is not live:
        return winner.mimetype, winner.listen()
    return live.mimetype, live.listen()


def _safe_unlink(path) -> None:
    try:
        Path(path).unlink()
    except OSError:
        pass


# -- metadata import (Deezer -> neezer rows) --------------------------


def find_local_track(sng_id) -> Track | None:
    """The already-imported Track for a Deezer id, looked up in the DB with no
    network call. Returns None if it was never imported.

    Track ids are a deterministic uuid5 of the Deezer id, so this is a direct
    primary-key lookup — it lets streaming serve archived audio even when Deezer
    is unreachable.
    """
    try:
        return Track[ids.track_uuid(sng_id)]
    except Track.DoesNotExist:
        return None


def import_track(provider, sng_id) -> Track:
    root = library.get_root_folder(provider.archive_dir)
    info = provider.get_track_info(sng_id)
    return library.upsert_track(info, root, provider.default_quality)


def import_album(provider, alb_id) -> list[Track]:
    root = library.get_root_folder(provider.archive_dir)
    tracks = provider.get_album_tracks(alb_id)
    return [library.upsert_track(t, root, provider.default_quality) for t in tracks]


def import_playlist_tracks(provider, playlist_id) -> list[Track]:
    """Import every track of a Deezer playlist (rows only, no ordering)."""
    root = library.get_root_folder(provider.archive_dir)
    tracks = provider.get_playlist_tracks(playlist_id)
    return [library.upsert_track(t, root, provider.default_quality) for t in tracks]


# -- cover art fallback for not-yet-archived Deezer entities -------------


def deezer_cover_path(provider, cache, eid: str):
    """Return a cached cover path fetched from Deezer for a Deezer id, or None.

    Resolves the id to an Album/Track (cover via md5 CDN), or an Artist/Playlist
    (image via Deezer's public image endpoint). Ids are type-namespaced uuid5,
    so only one lookup matches.
    """
    import uuid

    try:
        key = uuid.UUID(str(eid))
    except ValueError:
        return None

    data = None
    try:
        alb = Album[key]
        # Prefer the archived cover.jpg sidecar (served straight from disk, no
        # Deezer call) and only fall back to fetching from Deezer if absent.
        local = library.album_cover_file(alb)
        if local:
            return local
        data = provider.fetch_cover(alb.cover_md5) if alb.cover_md5 else None
    except Album.DoesNotExist:
        try:
            data = provider.fetch_image("artist", Artist[key].deezer_id)
        except Artist.DoesNotExist:
            try:
                data = provider.fetch_image("playlist", Playlist[key].deezer_id)
            except Playlist.DoesNotExist:
                try:
                    tr = Track[key]
                    data = (
                        provider.fetch_cover(tr.album.cover_md5)
                        if tr.album.cover_md5
                        else None
                    )
                except Track.DoesNotExist:
                    try:
                        ch = PodcastChannel[key]
                        # Archived art first: a show Deezer has dropped keeps its
                        # face, exactly like an album keeps its cover.jpg.
                        local = library.show_cover_file(provider.archive_dir, ch)
                        if local:
                            return local
                        data = (
                            provider.fetch_cover(ch.cover_art_md5)
                            if ch.cover_art_md5
                            else None
                        )
                    except PodcastChannel.DoesNotExist:
                        try:
                            ep = PodcastEpisode[key]
                            local = library.show_cover_file(
                                provider.archive_dir, ep.channel
                            )
                            if local:
                                return local
                            md5 = ep.image_md5 or ep.channel.cover_art_md5
                            data = provider.fetch_cover(md5) if md5 else None
                        except PodcastEpisode.DoesNotExist:
                            return None

    if not data:
        return None
    return cache.set(f"{eid}-deezer-cover", data)
