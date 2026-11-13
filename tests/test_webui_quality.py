# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""What a track payload says about the FILE behind it ("Qualité" column).

The lists used to carry nothing: `TrackRow.svelte` only knew YouTube's archived
bitrate, so every archived FLAC or uploaded MP3 left the column empty. The
payload now carries `quality`, `bitrate` and `suffix` — but only when the row's
own file is on disk, because `Track.bitrate` is the NOMINAL rate of the quality
that was ASKED for until the archive stamps the real one. A payload that showed
"FLAC" before the file arrived would be claiming what nobody hears yet.

The cost of that honesty is the point of the last test: the two fields come from
columns the list already reads (`path`, `last_modification`, `bitrate`), so the
statement count of a favourites page does not move.
"""

import collections
import contextlib
import os
import shutil
import tempfile
import unittest

import peewee

from neezer.config import DefaultConfig
from neezer.db import (
    Album,
    Artist,
    Folder,
    Playlist,
    PlaylistTrack,
    StarredTrack,
    Track,
    User,
    release_database,
)
from neezer.managers.user import UserManager
from neezer.web import create_application


@contextlib.contextmanager
def count_statements():
    counts = collections.Counter()
    original = peewee.Database.execute_sql

    def execute_sql(self, sql, *args, **kwargs):
        counts[sql.split(None, 1)[0].upper()] += 1
        return original(self, sql, *args, **kwargs)

    peewee.Database.execute_sql = execute_sql
    try:
        yield counts
    finally:
        peewee.Database.execute_sql = original


def raw(sid, title, art, alb, duration=200):
    return {
        "SNG_ID": str(sid),
        "SNG_TITLE": title,
        "ART_ID": art[0],
        "ART_NAME": art[1],
        "ALB_ID": alb[0],
        "ALB_TITLE": alb[1],
        "ALB_PICTURE": alb[2],
        "DURATION": duration,
        "TRACK_NUMBER": 1,
        "DISK_NUMBER": 1,
        "TRACK_TOKEN": f"tok{sid}",
    }


class QualityPayloadTestCase(unittest.TestCase):
    def setUp(self):
        self.__db = tempfile.mkstemp()
        self.__dir = tempfile.mkdtemp()
        self.archive = tempfile.mkdtemp()
        archive = self.archive
        db_path = self.__db[1]
        cache = self.__dir

        class Config(DefaultConfig):
            TESTING = True

            def __init__(self):
                super().__init__()
                self.BASE = dict(self.BASE, database_uri="sqlite:///" + db_path)
                self.WEBAPP = dict(
                    self.WEBAPP, cache_dir=cache, mount_webui=True, mount_api=True
                )
                self.DEEZER = dict(
                    getattr(self, "DEEZER", {}), archive_dir=archive, enabled=False
                )

        self.app = create_application(Config())
        # Deezer off: the lists below are the database's own answer.
        self.app.deezer = None
        UserManager.add("alice", "Alic3", admin=True)
        self.client = self.app.test_client()
        self._fill()

    def tearDown(self):
        release_database()
        shutil.rmtree(self.__dir, ignore_errors=True)
        shutil.rmtree(self.archive, ignore_errors=True)
        os.close(self.__db[0])
        os.remove(self.__db[1])

    def _fill(self):
        from neezer.deezer import library

        with self.app.app_context():
            root = library.get_root_folder(self.archive)
            art = ("11", "Édith")
            alb = ("21", "Album Un", "aa11")
            # Archived FLAC: the row's file is here, at its final path.
            self.flac = library.upsert_track(raw(1, "Été", art, alb), root)
            self.flac.path = os.path.join(self.archive, "Édith", "Album Un", "01 - Été.flac")
            self.flac.bitrate = 1000
            self.flac.last_modification = 1_700_000_000
            self.flac.save()
            # Imported, never archived: bitrate is FLAC's NOMINAL 1000 kbit/s,
            # not a measurement — the payload must say nothing.
            self.pending = library.upsert_track(raw(2, "Nuit", art, alb), root)
            self.assertEqual(self.pending.bitrate, 1000)
            self.assertEqual(self.pending.last_modification, 0)

            # An uploaded MP3: its own artist and album, the file already here.
            folder = Folder.create(
                root=False, name="Uploads", path=os.path.join(self.archive, "Uploads"),
                parent=root,
            )
            up_artist = Artist.create(name="Moi")
            up_album = Album.create(name="Maison", artist=up_artist)
            self.upload = Track.create(
                disc=1, number=1, title="Démo", duration=61,
                album=up_album, artist=up_artist, bitrate=320,
                path=os.path.join(self.archive, "Uploads", "demo.mp3"),
                last_modification=1_700_000_100, root_folder=root, folder=folder,
            )
            # YouTube keeps its own label: `source` + the archived file's REAL
            # rate, and no generic `quality` that would drop the number.
            self.yt_archived = library.upsert_track(raw(3, "Clip", art, alb), root)
            self.yt_archived.youtube_id = "vid1"
            self.yt_archived.deezer_id = None
            self.yt_archived.path = os.path.join(
                self.archive, "YouTube", "Édith", "Album Un", "Clip [vid1].opus"
            )
            self.yt_archived.bitrate = 133
            self.yt_archived.last_modification = 1_700_000_200
            self.yt_archived.save()
            self.yt_pending = library.upsert_track(raw(4, "À venir", art, alb), root)
            self.yt_pending.youtube_id = "vid2"
            self.yt_pending.deezer_id = None
            self.yt_pending.bitrate = 0
            self.yt_pending.last_modification = 0
            self.yt_pending.save()

            alice = User.get(name="alice")
            for k, t in enumerate(
                (self.flac, self.pending, self.upload, self.yt_archived, self.yt_pending)
            ):
                StarredTrack.create(user=alice, starred=t.id)
            self.playlist = Playlist.create(user=alice, name="Mix")
            for k, t in enumerate((self.flac, self.pending, self.upload)):
                PlaylistTrack.create(playlist=self.playlist, track=t.id, index=k)

    def _login(self):
        rv = self.client.post("/api/login", json={"username": "alice", "password": "Alic3"})
        self.assertEqual(rv.status_code, 200)

    def _favorites(self):
        self._login()
        rows = self.client.get("/api/me/favorites").get_json()["tracks"]
        return {r["title"]: r for r in rows}

    def test_an_archived_flac_says_what_it_is(self):
        row = self._favorites()["Été"]
        self.assertEqual(row["quality"], "FLAC")
        self.assertEqual(row["bitrate"], 1000)
        self.assertEqual(row["suffix"], "flac")

    def test_an_unarchived_track_claims_nothing(self):
        row = self._favorites()["Nuit"]
        self.assertNotIn("quality", row)
        self.assertNotIn("bitrate", row)
        self.assertNotIn("suffix", row)

    def test_an_upload_says_what_it_is(self):
        row = self._favorites()["Démo"]
        # A local row IS its file: imported from disk, rate read from its tags.
        self.assertEqual(row["quality"], "MP3 320")
        self.assertEqual(row["bitrate"], 320)
        self.assertEqual(row["suffix"], "mp3")

    def test_youtube_keeps_its_own_label(self):
        rows = self._favorites()
        archived = rows["Clip"]
        self.assertEqual(archived["source"], "youtube")
        # The player shows "OPUS 133" from these two; a generic quality would
        # replace it with a bare "OPUS" and lose the rate.
        self.assertNotIn("quality", archived)
        self.assertEqual(archived["bitrate"], 133)
        pending = rows["À venir"]
        self.assertNotIn("quality", pending)
        self.assertEqual(pending["bitrate"], 0)

    def test_the_playlist_list_carries_the_same_fields(self):
        self._login()
        rows = self.client.get(f"/api/playlist/{self.playlist.id}").get_json()["tracks"]
        by_title = {r["title"]: r for r in rows}
        self.assertEqual(by_title["Été"]["quality"], "FLAC")
        self.assertEqual(by_title["Démo"]["quality"], "MP3 320")
        self.assertNotIn("quality", by_title["Nuit"])

    def test_me_local_carries_the_upload_quality(self):
        self._login()
        rows = self.client.get("/api/me/local").get_json()["tracks"]
        by_title = {r["title"]: r for r in rows}
        self.assertEqual(by_title["Démo"]["quality"], "MP3 320")
        self.assertEqual(by_title["Démo"]["bitrate"], 320)
        self.assertEqual(by_title["Démo"]["suffix"], "mp3")

    def test_the_tuple_and_model_paths_agree(self):
        from neezer import webui

        with self.app.test_request_context():
            q = webui._db_playlist_track_rows(self.playlist)
            fast = webui._db_tracks(q)
            slow = webui._db_tracks(list(q))
        self.assertEqual(fast, slow)

    def test_the_quality_costs_no_statement(self):
        self._login()
        with count_statements() as counts:
            rows = self.client.get("/api/me/favorites").get_json()["tracks"]
        self.assertEqual(len(rows), 5)
        # The session, the star list, its credits — exactly what the favourites
        # page cost before the quality fields existed (tests/test_webui_lists.py
        # pins the same three).
        self.assertLessEqual(counts["SELECT"], 3, counts)


if __name__ == "__main__":
    unittest.main()
