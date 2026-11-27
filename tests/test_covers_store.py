# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Pochettes rangées dans le store de l'archive (``neezer/deezer/coverstore.py``).

Ce que ces tests prouvent, dans l'ordre du besoin du propriétaire : le layout
est déterministe et confiné, une entrée de cache existante est COPIÉE dans le
store (et une seconde passe ne fait rien), et l'image embarquée d'un FLAC
archivé devient le ``cover.jpg`` du dossier — un vrai ffmpeg, une vraie image,
pour que le job soit mesuré sur ce qu'il lit en production.
"""

import os
import os.path
import shutil
import subprocess
import tempfile
import unittest

from neezer.cache import Cache
from neezer.db import Album, Folder, Meta, Track
from neezer.deezer import coverstore, ids, library

from .testbase import TestBase

HAVE_FFMPEG = bool(shutil.which("ffmpeg"))


def _need_ffmpeg():
    if HAVE_FFMPEG:
        return
    if os.environ.get("NS_REQUIRE_FFMPEG") == "1":
        raise RuntimeError("NS_REQUIRE_FFMPEG=1 but ffmpeg is not on PATH")
    raise unittest.SkipTest("ffmpeg not installed")


def _raw_track(sid, alb="10", pic="covermd5"):
    return {
        "SNG_ID": str(sid),
        "SNG_TITLE": f"T{sid}",
        "ART_ID": "1",
        "ART_NAME": "Artist",
        "ALB_ID": str(alb),
        "ALB_TITLE": f"Album {alb}",
        "ALB_PICTURE": pic,
        "DURATION": "30",
        "TRACK_NUMBER": "1",
        "DISK_NUMBER": "1",
    }


def _jpeg() -> bytes:
    import io

    from PIL import Image

    buf = io.BytesIO()
    Image.new("RGB", (8, 8), (200, 30, 30)).save(buf, "JPEG")
    return buf.getvalue()


def _archived_track(archive_dir, sid="1", alb="10", with_art=True):
    """A Track row whose FLAC is on disk, with a real embedded cover."""
    root = library.get_root_folder(archive_dir)
    track = library.upsert_track(_raw_track(sid, alb), root, "FLAC")
    os.makedirs(os.path.dirname(track.path), exist_ok=True)
    subprocess.run(
        ["ffmpeg", "-v", "error", "-nostdin", "-y", "-f", "lavfi",
         "-i", "sine=frequency=440:duration=1", track.path],
        check=True,
    )
    if with_art:
        from mutagen.flac import FLAC, Picture

        audio = FLAC(track.path)
        picture = Picture()
        picture.type = 3
        picture.mime = "image/jpeg"
        picture.data = _jpeg()
        audio.add_picture(picture)
        audio.save()
    track.last_modification = 1700000000  # what archive._finalize_archive stamps
    track.save(only=[Track.last_modification])
    return Track[track.id]


class CoverStoreTestCase(TestBase):
    def setUp(self):
        super().setUp()
        self.archive = tempfile.mkdtemp()
        self.cache_dir = tempfile.mkdtemp()
        self.cache = Cache(self.cache_dir, 10 * 1024 * 1024)
        coverstore.reset_for_tests()
        coverstore._counts.update(coverstore._new_counts())

    def tearDown(self):
        coverstore.reset_for_tests()
        shutil.rmtree(self.archive, ignore_errors=True)
        shutil.rmtree(self.cache_dir, ignore_errors=True)
        super().tearDown()

    def _track(self, sid="1", alb="10"):
        root = library.get_root_folder(self.archive)
        return library.upsert_track(_raw_track(sid, alb), root, "FLAC")

    # -- layout ----------------------------------------------------------

    def test_layout_is_keyed_by_the_entity_uuid(self):
        track = self._track()
        path = coverstore.store_path(self.archive, "album", track.album.id)
        self.assertTrue(path.startswith(os.path.realpath(self.archive)))
        self.assertEqual(
            os.path.relpath(path, os.path.realpath(self.archive)),
            os.path.join("Covers", "album", f"{track.album.id}.jpg"),
        )
        # The same entity always names the same file: a re-fetch rewrites
        # nothing, so a scrolling shelf costs a stat, not bytes.
        self.assertEqual(path, coverstore.store_path(self.archive, "album", track.album.id))

    def test_no_client_supplied_id_or_kind_can_name_a_path(self):
        self.assertIsNone(coverstore.store_path(self.archive, "album", "../../etc/passwd"))
        self.assertIsNone(coverstore.store_path(self.archive, "album", "not-a-uuid"))
        self.assertIsNone(coverstore.store_path(self.archive, "../album", ids.album_uuid("10")))
        self.assertIsNone(coverstore.store_path(None, "album", ids.album_uuid("10")))

    def test_saving_is_atomic_and_never_overwrites(self):
        album_id = ids.album_uuid("10")
        first = coverstore.save_cover(self.archive, "album", album_id, b"first")
        self.assertTrue(first and os.path.isfile(first))
        again = coverstore.save_cover(self.archive, "album", album_id, b"second")
        self.assertEqual(first, again)
        with open(first, "rb") as fh:
            self.assertEqual(fh.read(), b"first")
        self.assertEqual(coverstore.local_cover(self.archive, "album", album_id), first)
        self.assertEqual(
            [n for n in os.listdir(os.path.dirname(first)) if n.endswith(".part")], []
        )

    # -- migration of what is already cached -----------------------------

    def test_cached_covers_are_copied_into_the_store_once(self):
        track = self._track()
        album = track.album
        artist = track.artist
        self.cache.set(f"{album.id}-deezer-cover", b"ALBUMJPEG")
        self.cache.set(f"{artist.id}-deezer-cover", b"ARTISTJPEG")
        # A track key holds the same sleeve as its album: whichever is walked
        # first, the album's store file ends up with the album's bytes.
        self.cache.set(f"deezer-cover-{track.id}", b"ALBUMJPEG")

        counts = coverstore.migrate_cache(self.cache, self.archive)
        self.assertEqual(counts["moved"], 2)
        with open(coverstore.local_cover(self.archive, "album", album.id), "rb") as fh:
            self.assertEqual(fh.read(), b"ALBUMJPEG")
        with open(coverstore.local_cover(self.archive, "artist", artist.id), "rb") as fh:
            self.assertEqual(fh.read(), b"ARTISTJPEG")
        # A track shares its album's sleeve: one file, not two.
        self.assertEqual(
            coverstore.local_cover(self.archive, "track", track.id),
            None,
        )

        # Idempotent: the second pass only confirms what is there.
        counts = coverstore.migrate_cache(self.cache, self.archive)
        self.assertEqual((counts["moved"], counts["kept"]), (0, 3))

    def test_derived_cache_entries_and_dead_rows_are_left_alone(self):
        self.cache.set("localcover-deadbeef", b"X")
        self.cache.set(f"{ids.album_uuid('999')}-deezer-cover", b"X")
        self.cache.set(f"{ids.album_uuid('10')}-cover-200", b"X")
        counts = coverstore.migrate_cache(self.cache, self.archive)
        self.assertEqual(counts["moved"], 0)
        self.assertEqual(counts["skipped"], 1)  # the album row does not exist
        self.assertFalse(os.path.isdir(os.path.join(self.archive, "Covers")))

    # -- archived audio --------------------------------------------------

    def test_an_archived_flac_hands_its_embedded_cover_to_the_store(self):
        _need_ffmpeg()
        track = _archived_track(self.archive)
        album = Album[track.album_id]
        self.assertIsNone(library.album_cover_file(album))

        self.assertIsNone(coverstore._migrate_album(album))

        cover_path = library.album_cover_file(album)
        self.assertTrue(cover_path and os.path.isfile(cover_path))
        self.assertEqual(os.path.basename(cover_path), "cover.jpg")
        self.assertEqual(Folder[track.folder_id].cover_art, "cover.jpg")
        with open(cover_path, "rb") as fh:
            self.assertEqual(fh.read(), _jpeg())
        self.assertEqual(coverstore._counts["flac"], 1)

        # Second run: nothing left to do, nothing rewritten.
        before = os.stat(cover_path).st_mtime_ns
        self.assertIsNone(coverstore._migrate_album(album))
        self.assertEqual(os.stat(cover_path).st_mtime_ns, before)

    def test_a_track_without_embedded_art_leaves_the_disk_alone(self):
        _need_ffmpeg()
        track = _archived_track(self.archive, with_art=False)
        album = Album[track.album_id]
        self.assertIsNone(coverstore._migrate_album(album))
        self.assertIsNone(library.album_cover_file(album))

    def test_the_job_resumes_where_a_restart_cut_it(self):
        _need_ffmpeg()
        _archived_track(self.archive, sid="1", alb="10")
        _archived_track(self.archive, sid="2", alb="20")
        albums = [Album[ids.album_uuid("10")], Album[ids.album_uuid("20")]]

        # One album per page, so the run really stops between two of them —
        # what a container restart does to a job of thousands.
        from neezer.libjob import LibraryJob

        job = LibraryJob(
            "covers-store", Album.select(), coverstore._migrate_album,
            cursor_key=coverstore.CURSOR_KEY, page_size=1,
        )
        first = job.run(limit=1)
        self.assertFalse(first["finished"])
        self.assertEqual(first["done"], 1)
        self.assertEqual(sum(bool(library.album_cover_file(a)) for a in albums), 1)
        self.assertIsNotNone(Meta.get_or_none(Meta.key == coverstore.CURSOR_KEY))

        resumed = coverstore.make_job().run()
        self.assertTrue(resumed["finished"])
        self.assertEqual(sum(bool(library.album_cover_file(a)) for a in albums), 2)
        self.assertIsNone(Meta.get_or_none(Meta.key == coverstore.CURSOR_KEY))


if __name__ == "__main__":
    unittest.main()
