# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Pochettes rangées avec les sons, pas seulement dans le cache.

Une pochette que ce serveur a récupérée est une donnée à lui, comme le FLAC :
elle doit vivre dans le store de l'archive (``DEEZER.archive_dir``, la même
racine que ``archive.py``), être servie de là ensuite sans un seul appel
fournisseur, et survivre à un prune du cache comme à un redémarrage. Avant ce
module, les pochettes d'album archivé avaient bien leur ``cover.jpg`` à côté du
son ; celles d'un artiste, d'une playlist, d'une émission ou d'un album pas
encore archivé ne vivaient que dans ``WEBAPP.cache_dir`` — un prune les
effaçait, et le serveur les récupérait de Deezer au premier affichage suivant.

Le layout est déterministe, dérivé de l'identité serveur (uuid5, ``ids.py``),
jamais d'un id ou d'un chemin fourni par le client ::

    <archive_dir>/Covers/album/<uuid>.jpg
    <archive_dir>/Covers/artist/<uuid>.jpg
    <archive_dir>/Covers/playlist/<uuid>.jpg
    <archive_dir>/Covers/show/<uuid>.jpg    # émission, et chaque épisode
    <archive_dir>/Covers/track/<uuid>.jpg   # piste sans ligne d'album

Une piste résout vers l'album (même fichier que la pochette de l'album) ; un
album déjà archivé garde son ``cover.jpg`` dans son dossier, consulté avant
``Covers/`` : aucun doublon pour ces albums, et rien à déplacer.

La migration (rule 7) est un job de fond ``libjob.LibraryJob`` : il copie les
entrées ``*-deezer-cover`` déjà en cache, puis extrait l'image embarquée des FLAC
archivés qui n'ont pas encore de ``cover.jpg``. Il est idempotent, checkpointé
(reprise après restart), renicé et il attend qu'aucune lecture ne soit en cours
entre deux albums (rule 1). Rien ici ne supprime une pochette : comme le reste
de l'archive, seul ``deezer/cleanup.py`` libère de la place, et il ne touche
qu'à l'audio.
"""

from __future__ import annotations

import contextlib
import logging
import os
import os.path
import threading
import time
import uuid

from ..db import Album, Artist, Meta, Playlist, PodcastChannel, PodcastEpisode, Track
from ..libjob import LibraryJob
from . import ids, library

logger = logging.getLogger(__name__)

STORE_DIR_NAME = "Covers"
#: Kinds are a closed list: a kind never comes from a request or a database
#: value, so it can never name a directory of its own.
KINDS = ("album", "artist", "playlist", "show", "track")
#: Deezer's image CDN and its public image endpoint both answer JPEG.
STORE_SUFFIX = ".jpg"

CURSOR_KEY = "covers_store"
#: Written when a full pass has finished; its presence is what stops the job
#: from rescanning every album on every boot.
DONE_KEY = "covers_store_done"

_lock = threading.Lock()
_job = None
_counts = {}
#: Why a cached sleeve could not be moved (bounded). Album failures live on the
#: job's own snapshot; both are served by ``status()`` so a failure always names
#: its cause, never just a count (rule 2).
_failures = []
FAILURE_SAMPLE_MAX = 40


def _new_counts():
    # ``moved``/``kept``/``skipped``/``failed``: cache entries. ``flac``: albums
    # whose cover.jpg was written out of their archived audio.
    return {"moved": 0, "kept": 0, "skipped": 0, "failed": 0, "flac": 0}


def _archive_root(provider) -> str | None:
    root = getattr(provider, "archive_dir", None)
    return root or None


# -- identity ---------------------------------------------------------------


def _as_uuid(value):
    try:
        return uuid.UUID(str(value))
    except (ValueError, TypeError, AttributeError):
        return None


def _row(model, key):
    """The row ``key`` names in ``model``, or None — no exception as control flow."""
    try:
        return model[key]
    except model.DoesNotExist:
        return None


def target(eid):
    """``(kind, uuid, row)`` for a Deezer entity id, or None.

    Mirrors the lookup order of ``archive.deezer_cover_path`` so the file that
    is written is always the file that is read. Numeric Deezer track ids are
    accepted too (Subsonic clients ask for those): they resolve to the archived
    album's file, one copy per sleeve. A numeric id with no Track row yet (a
    search hit somebody just looked at) still gets a durable home, keyed by this
    server's deterministic uuid5 for that Deezer id — that is what "every cover
    the server fetches is stored" means. ``row`` is None in that one case.
    """
    key = _as_uuid(eid)
    if key is not None:
        album = _row(Album, key)
        if album is not None:
            return ("album", key, album)
        artist = _row(Artist, key)
        if artist is not None:
            return ("artist", key, artist)
        playlist = _row(Playlist, key)
        if playlist is not None:
            return ("playlist", key, playlist)
        track = _row(Track, key)
        if track is not None:
            # An archived album's art is its album's: the track's sleeve and the
            # album's are one file, not two.
            if track.album is not None:
                return ("album", track.album.id, track.album)
            return ("track", track.id, track)
        channel = _row(PodcastChannel, key)
        if channel is not None:
            return ("show", key, channel)
        episode = _row(PodcastEpisode, key)
        if episode is not None:
            return ("show", key, episode)
        return None
    # A numeric Deezer id is how several Subsonic clients name a track.
    if str(eid).isdigit():
        track = Track.get_or_none(Track.deezer_id == str(eid))
        if track is not None:
            if track.album is not None:
                return ("album", track.album.id, track.album)
            return ("track", track.id, track)
        # Not imported yet: its own deterministic key, so the bytes still land
        # in the store instead of dying with the cache.
        return ("track", ids.track_uuid(str(eid)), None)
    return None


def _valid_kind(kind) -> bool:
    return kind in KINDS


def store_path(archive_dir, kind, key) -> str | None:
    """The store file for ``kind``/``key``; None when the inputs are unusable."""
    if not archive_dir or not _valid_kind(kind):
        return None
    key = _as_uuid(key)
    if key is None:
        return None
    return os.path.join(
        os.path.abspath(os.path.expanduser(archive_dir)),
        STORE_DIR_NAME,
        kind,
        f"{key}{STORE_SUFFIX}",
    )


def local_cover(archive_dir, kind, key) -> str | None:
    """The stored file, if this server already holds that sleeve."""
    path = store_path(archive_dir, kind, key)
    return path if path and os.path.isfile(path) else None


def save_cover(archive_dir, kind, key, data) -> str | None:
    """Write ``data`` in the store at most once, atomically; the path or None.

    No-overwrite on purpose: the same sleeve is written on the first fetch and
    every later read is a stat, so a scrolling shelf never rewrites bytes. An
    unwritable store (read-only mount, full disk) returns None and lets the
    caller fall back to the cache — an archive store that refuses is never a
    reason to blank a cover.
    """
    if not data:
        return None
    path = store_path(archive_dir, kind, key)
    if path is None:
        return None
    try:
        if os.path.isfile(path):
            return path
        os.makedirs(os.path.dirname(path), exist_ok=True)
        tmp = f"{path}.{uuid.uuid4().hex}.part"
        try:
            with open(tmp, "wb") as fh:
                fh.write(data)
            os.replace(tmp, path)
        finally:
            # The staged file is a temp name of ours; a remove that fails (a
            # race with another writer) leaves nothing anybody serves.
            with contextlib.suppress(OSError):
                os.remove(tmp)
    except OSError:
        logger.debug("Could not store cover at %s", path, exc_info=True)
        return None
    return path


# -- what the store already holds -------------------------------------------


def store_stats(archive_dir) -> dict:
    """Files and bytes held under ``Covers/`` — the number the PR reports."""
    files = 0
    total = 0
    if not archive_dir:
        return {"files": 0, "bytes": 0}
    root = os.path.join(os.path.abspath(os.path.expanduser(archive_dir)), STORE_DIR_NAME)
    with contextlib.suppress(OSError):
        for dirpath, _dirnames, filenames in os.walk(root):
            for name in filenames:
                if not name.endswith(STORE_SUFFIX):
                    continue
                try:
                    size = os.path.getsize(os.path.join(dirpath, name))
                except OSError:  # deleted while we walk: it holds nothing now
                    size = None
                if size is not None:
                    total += size
                    files += 1
    return {"files": files, "bytes": total}


# -- fetching bytes for a target --------------------------------------------


def _track_for(eid):
    """The Track row an id names, without a network call (None when unknown)."""
    key = _as_uuid(eid)
    if key is not None:
        return Track.get_or_none(Track.id == key)
    if str(eid).isdigit():
        return Track.get_or_none(Track.deezer_id == str(eid))
    return None


def fetch_for(provider, kind, row, eid=None) -> bytes | None:
    """The bytes of ``row``'s art from the provider, or None. Never a verdict.

    Every provider call here is already fail-soft (it returns None on a
    transport failure), so an outage costs a missing cover today and the same
    attempt tomorrow — nothing is recorded against the entity.
    """
    from .archive import cover_md5

    if kind == "album":
        raw = str(row.cover_md5 or "").strip()
        md5 = raw if raw and set(raw) != {"0"} else None
        via_track = _track_for(eid) if eid is not None else None
        if not md5 and via_track is not None and via_track.deezer_id:
            # The track that asked is the cheapest way to its sleeve, and it is
            # the call the track route has always made. It also repairs a row
            # imported from a listing without ALB_PICTURE (PR #469).
            try:
                payload = provider.get_track_info(via_track.deezer_id) or {}
            except Exception:  # a provider failure says nothing about this album
                logger.debug("Track art lookup failed", exc_info=True)
                payload = {}
            if isinstance(payload, dict):
                md5 = payload.get("ALB_PICTURE")
                covered_md5 = str(md5 or "").strip()
                if covered_md5 and set(covered_md5) != {"0"} and not row.cover_md5:
                    row.cover_md5 = covered_md5
                    row.save(only=[Album.cover_md5])
        if not md5 and via_track is None:
            # An album asked for by its own id, with no hash yet: its page is
            # the one call that can repair the row.
            md5 = cover_md5(row, provider)
        return provider.fetch_cover(md5) if md5 else None
    if kind == "artist":
        return provider.fetch_image("artist", row.deezer_id)
    if kind == "playlist":
        return provider.fetch_image("playlist", row.deezer_id)
    if kind == "show":
        md5 = getattr(row, "image_md5", None) or getattr(row, "cover_art_md5", None)
        if not md5:
            channel = getattr(row, "channel", None)
            md5 = getattr(channel, "cover_art_md5", None) if channel else None
        return provider.fetch_cover(md5) if md5 else None
    if kind == "track":
        md5 = None
        info = getattr(row, "deezer_id", None) or (
            str(eid) if eid is not None and str(eid).isdigit() else None
        )
        if info:
            try:
                payload = provider.get_track_info(info) or {}
            except Exception:  # a provider failure says nothing about this track
                logger.debug("Track art lookup failed", exc_info=True)
                payload = {}
            if isinstance(payload, dict):
                md5 = payload.get("ALB_PICTURE")
        return provider.fetch_cover(md5) if md5 else None
    return None


# -- migrating what already exists (rule 7) ---------------------------------


def _cache_entity(name: str) -> str | None:
    """The entity a legacy cover cache key names, if it names one.

    Two shapes existed: ``<uuid>-deezer-cover`` (albums and the non-album
    entities) and ``deezer-cover-<track id>`` (a track by Deezer id).
    Thumbnails (``<id>-cover-<size>``) and extracted local art
    (``localcover-<id>``) are derived data: they stay in the cache and are
    regenerated from the store on demand.
    """
    if name.endswith("-deezer-cover"):
        value = name[: -len("-deezer-cover")]
        return value if _as_uuid(value) is not None else None
    if name.startswith("deezer-cover-"):
        value = name[len("deezer-cover-") :]
        if _as_uuid(value) is not None:
            return value
        track = Track.get_or_none(Track.deezer_id == value)
        return str(track.id) if track is not None else None
    return None


def migrate_cache(cache, archive_dir, counts=None, failures=None) -> dict:
    """Copy the covers already in the cache into the store. Idempotent."""
    counts = counts if counts is not None else _new_counts()
    root = getattr(cache, "_cache_dir", None)
    if not root or not archive_dir:
        return counts
    try:
        entries = sorted(os.scandir(root), key=lambda e: e.name)
    except OSError:
        logger.debug("Could not read the cover cache", exc_info=True)
        return counts
    for entry in entries:
        if not entry.is_file():
            continue
        eid = _cache_entity(entry.name)
        if eid is None:
            continue
        found = target(eid)
        if found is None:
            # The row is gone; the cache entry has nothing left to describe.
            counts["skipped"] += 1
            continue
        kind, key, _row = found
        if local_cover(archive_dir, kind, key):
            counts["kept"] += 1
            continue
        try:
            with open(entry.path, "rb") as fh:
                data = fh.read()
        except OSError as exc:
            counts["failed"] += 1
            if failures is not None:
                failures.append({"id": eid, "reason": f"{type(exc).__name__}: {exc}"})
            continue
        if save_cover(archive_dir, kind, key, data):
            counts["moved"] += 1
        else:
            counts["failed"] += 1
            if failures is not None:
                failures.append({"id": eid, "reason": "could not write the store file"})
    return counts


def _embedded_art(path) -> bytes | None:
    """The first picture embedded in an archived audio file, or None."""
    try:
        import mediafile

        mf = mediafile.MediaFile(path)
        for image in mf.images or ():
            data = getattr(image, "data", None)
            if data:
                return data
    except Exception:  # a file we cannot read has no extractable art
        logger.debug("Could not read embedded art from %s", path, exc_info=True)
    return None


def _migrate_album(album):
    """Write an archived album's cover.jpg from its audio. Reason or None."""
    from .workload import quiet_wait

    if library.album_cover_file(album):
        return None
    track = (
        album.tracks.where(Track.last_modification > 0)
        .order_by(Track.number)
        .first()
    )
    if track is None or not track.path or not os.path.isfile(track.path):
        return None
    quiet_wait(2.0)  # a person's play always wins over this extraction
    data = _embedded_art(track.path)
    if not data:
        return None
    # cover.jpg beside the tracks IS the store for an archived album: the
    # Subsonic side, the web side and the archive's own metadata all point at it.
    if library.save_album_cover(track.folder, data) is None:
        raise OSError("could not write cover.jpg")
    _counts["flac"] = _counts.get("flac", 0) + 1
    return None


def make_job(force=False):
    return LibraryJob(
        "covers-store",
        Album.select(),
        _migrate_album,
        cursor_key=CURSOR_KEY,
        force=force,
    )


# -- running it -------------------------------------------------------------

#: Let the server finish coming up before a library job takes the disk.
RESUME_DELAY = 45


def _mark_done():
    row = Meta.get_or_none(Meta.key == DONE_KEY)
    if row is None:
        Meta.create(key=DONE_KEY, value="done")
    elif row.value != "done":
        row.value = "done"
        row.save(only=[Meta.value])


def _already_done() -> bool:
    return Meta.get_or_none(Meta.key == DONE_KEY) is not None


def _run(app, job, cache, archive_dir):
    from ..db import close_connection, open_connection

    with app.app_context():
        try:
            open_connection(reuse=True)
            try:
                migrate_cache(cache, archive_dir, _counts, _failures)
            except Exception:  # the FLAC pass is still worth doing
                logger.warning("Cover cache migration failed", exc_info=True)
            job.run()
            if job.snapshot()["finished"]:
                _mark_done()
                logger.info(
                    "Covers stored with the archive: %s from the cache, %s already "
                    "there, %s from archived audio, %s failed",
                    _counts["moved"], _counts["kept"], _counts["flac"],
                    _counts["failed"],
                )
        except Exception:
            logger.warning("Cover store migration crashed", exc_info=True)
        finally:
            try:
                close_connection()
            except Exception:  # nosec B110 - the run is over; a close that fails has nothing to report
                pass


def status(provider=None) -> dict:
    with _lock:
        job = _job
        counts = dict(_counts) or _new_counts()
        cache_failures = list(_failures[:FAILURE_SAMPLE_MAX])
    snap = job.snapshot() if job is not None else {
        "name": "covers-store", "running": False, "done": 0, "failed": 0,
        "failures": [], "failure_reasons": {}, "error": None, "finished": False,
    }
    archive_dir = _archive_root(provider)
    return {
        **snap,
        "counts": counts,
        "cache_failures": cache_failures,
        "store": store_stats(archive_dir),
        "complete": bool(snap["finished"]) or _safe(_already_done, False),
    }


def _safe(fn, default):
    """A status read must never 500 because the database is mid-migration."""
    try:
        return fn()
    except Exception:
        logger.debug("Cover store status read failed", exc_info=True)
        return default


def running() -> bool:
    return bool(_job is not None and _job.snapshot()["running"])


def start(app, force=False):
    """Start the migration on a daemon thread; a no-op while one runs."""
    global _job, _counts, _failures
    with _lock:
        if _job is not None and _job.snapshot()["running"]:
            return False
        provider = getattr(app, "deezer", None)
        archive_dir = _archive_root(provider)
        if not archive_dir:
            return False
        _job = make_job(force)
        _counts = _new_counts()
        _failures = []
        job = _job
        job._stats["running"] = True  # a poll right after the POST already says so
    t = threading.Thread(
        target=_run,
        args=(app, job, getattr(app, "cache", None), archive_dir),
        name="covers-store",
        daemon=True,
    )
    t.start()
    return True


def autostart(app):
    """On boot: resume a cut run, or do the one-time pass on an existing server."""

    def go():
        time.sleep(RESUME_DELAY)
        with app.app_context():
            try:
                from ..db import close_connection, open_connection

                open_connection(reuse=True)
                try:
                    pending = make_job()._checkpoint_read() is not None
                    done = _already_done()
                finally:
                    try:
                        close_connection()
                    except Exception:  # nosec B110 - the thread is done with the database either way
                        pass
            except Exception:
                logger.debug("Could not read the cover store state", exc_info=True)
                return
        if pending or not done:
            logger.info(
                "%s the cover store migration",
                "Resuming" if pending else "Starting",
            )
            start(app)

    threading.Thread(target=go, name="covers-store-resume", daemon=True).start()


def reset_for_tests():
    """Drop the module-level job/counters (tests only)."""
    global _job, _counts, _failures
    with _lock:
        _job = None
        _counts = {}
        _failures = []
