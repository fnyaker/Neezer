# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Regression tests for the follow-up security findings (S-01, S-07)."""

import io
import os
import shutil
import tempfile
import unittest
from unittest import mock

from neezer.db import (
    Meta,
    ProviderAccount,
    ProviderGrant,
    Track,
    User,
    release_database,
)
from neezer.managers.user import UserManager
from neezer.web import create_application


class _Tag:
    genre = None
    disc = 1
    track = 1
    year = None
    length = 10.0
    bitrate = 128000
    images = []

    def __init__(self, title, artist, album):
        self.title = title
        self.artist = artist
        self.albumartist = None
        self.album = album


class _FakeTag(_Tag):
    def __init__(self):
        super().__init__("Shared song", "Shared artist", "Shared album")


class UploadMetadataLeakTestCase(unittest.TestCase):
    """S-01: a private upload's metadata and cover never reach another account."""

    def setUp(self):
        from tests.testbase import TestConfig
        from neezer.deezer import library, local

        self.__db = tempfile.mkstemp()
        self.__dir = tempfile.mkdtemp()
        self.archive = tempfile.mkdtemp()
        cfg = TestConfig(True, True)
        cfg.BASE["database_uri"] = "sqlite:///" + self.__db[1]
        cfg.WEBAPP["cache_dir"] = self.__dir
        cfg.DEEZER["archive_dir"] = self.archive
        self.app = create_application(cfg)
        self.app.config["DEEZER"]["archive_dir"] = self.archive

        UserManager.add("alice", "Alic3", admin=True)
        self.bob = UserManager.add("bob", "B0bpass")
        self.mallory = UserManager.add("mallory", "M4llory")
        self.client = self.app.test_client()

        with open("tests/assets/formats/silence.mp3", "rb") as fh:
            payload = fh.read()
        self._login("bob", "B0bpass")
        from neezer.deezer import analysis
        from neezer.enrich import queue as enrich_queue

        with mock.patch.object(analysis, "queue_analysis", lambda *a, **k: None), \
                mock.patch.object(enrich_queue, "queue_track", lambda *a, **k: None):
            rv = self.client.post(
                "/api/upload",
                data={"files": (io.BytesIO(payload), "bob-private.mp3")},
                content_type="multipart/form-data",
            )
        self.assertEqual(rv.status_code, 200)
        self.assertEqual(rv.get_json()["count"], 1)
        self.client.post("/api/logout")

        self.private = Track.get(Track.owner == self.bob.id)
        self.assertEqual(self.private.album.name, "Awesome album")
        self.assertTrue(self.private.has_art)

        root = library.get_root_folder(self.archive)
        shared = os.path.join(self.archive, "shared.mp3")
        with open("tests/assets/formats/silence.mp3", "rb") as fh, open(shared, "wb") as out:
            out.write(fh.read())
        orig = local._load_tag
        try:
            local._load_tag = lambda p: _FakeTag()
            self.shared = local.import_local_file(shared, root)
            for name, tag, owner in (
                ("mixed-private.mp3", ("Private mixed", "Mixed artist", "Mixed album"), self.bob),
                ("mixed-shared.mp3", ("Shared mixed", "Mixed artist", "Mixed album"), None),
            ):
                path = os.path.join(self.archive, name)
                with open(path, "wb") as out:
                    out.write(payload)
                local._load_tag = lambda p, t=tag: _Tag(*t)
                track = local.import_local_file(path, root, owner=owner)
                if owner is None:
                    self.mixed_shared = track
                else:
                    self.mixed_private = track
        finally:
            local._load_tag = orig

    def tearDown(self):
        release_database()
        for d in (self.__dir, self.archive):
            shutil.rmtree(d, ignore_errors=True)
        os.close(self.__db[0])
        os.remove(self.__db[1])

    def _login(self, name, password):
        return self.client.post(
            "/api/login", json={"username": name, "password": password}
        )

    def _rest(self, user, password, endpoint, **params):
        args = {"u": user, "p": password, "c": "tests", "f": "json"}
        args.update(params)
        return self.client.get(f"/rest/{endpoint}", query_string=args)

    def _error(self, rv):
        body = rv.get_json()["subsonic-response"]
        self.assertEqual(body["status"], "failed")
        return body["error"]["code"]

    def test_non_owner_reads_no_metadata(self):
        tid, aid, arid = str(self.private.id), str(self.private.album.id), str(self.private.artist.id)

        self.assertEqual(self._error(self._rest("mallory", "M4llory", "getSong", id=tid)), 70)
        self.assertEqual(self._error(self._rest("mallory", "M4llory", "getAlbum", id=aid)), 70)
        self.assertEqual(self._error(self._rest("mallory", "M4llory", "getArtist", id=arid)), 70)

        artists = self._rest("mallory", "M4llory", "getArtists").get_json()["subsonic-response"]
        names = [
            a["name"]
            for index in artists.get("artists", {}).get("index", [])
            for a in index.get("artist", [])
        ]
        self.assertNotIn("Some artist", names)
        self.assertIn("Shared artist", names)

    def test_non_owner_gets_no_cover_bytes(self):
        tid = str(self.private.id)
        aid = str(self.private.album.id)

        for entity in (tid, aid):
            rv = self._rest("mallory", "M4llory", "getCoverArt", id=entity)
            self.assertEqual(self._error(rv), 70, entity)
            self.assertNotIn(b"\x89PNG", rv.data)

    def test_non_owner_gets_no_csv_row(self):
        self._login("mallory", "M4llory")
        rv = self.client.get(f"/api/services/list/album/{self.private.album.id}?fmt=csv")
        self.assertEqual(rv.status_code, 404)
        self.assertNotIn(b"silence", rv.data)

    def test_a_mixed_album_shows_only_the_visible_tracks(self):
        aid = str(self.mixed_shared.album.id)
        self.assertEqual(self.mixed_private.album.id, self.mixed_shared.album.id)

        body = self._rest("mallory", "M4llory", "getAlbum", id=aid).get_json()["subsonic-response"]
        self.assertEqual(body["status"], "ok")
        self.assertEqual(body["album"]["songCount"], 1)
        self.assertEqual([s["title"] for s in body["album"]["song"]], ["Shared mixed"])

        body = self._rest(
            "mallory", "M4llory", "getAlbumList2", type="alphabeticalByName"
        ).get_json()["subsonic-response"]
        names = [a["name"] for a in body.get("albumList2", {}).get("album", [])]
        self.assertIn("Mixed album", names)
        self.assertNotIn("Awesome album", names)

        body = self._rest("bob", "B0bpass", "getAlbum", id=aid).get_json()["subsonic-response"]
        self.assertEqual(body["album"]["songCount"], 2)
        self.assertEqual(
            sorted(s["title"] for s in body["album"]["song"]),
            ["Private mixed", "Shared mixed"],
        )

    def test_owner_and_admin_still_read_it(self):
        tid, aid, arid = str(self.private.id), str(self.private.album.id), str(self.private.artist.id)

        for name, password in (("bob", "B0bpass"), ("alice", "Alic3")):
            rv = self._rest(name, password, "getSong", id=tid)
            body = rv.get_json()["subsonic-response"]
            self.assertEqual(body["status"], "ok", name)
            self.assertEqual(body["song"]["title"], "[silence]")
            self.assertTrue(body["song"]["path"].startswith("Uploads/"))

            body = self._rest(name, password, "getAlbum", id=aid).get_json()["subsonic-response"]
            self.assertEqual(body["status"], "ok", name)
            self.assertEqual(body["album"]["songCount"], 1)
            self.assertEqual(len(body["album"]["song"]), 1)

            body = self._rest(name, password, "getArtist", id=arid).get_json()["subsonic-response"]
            self.assertEqual(body["status"], "ok", name)
            self.assertEqual(len(body["artist"]["album"]), 1)

            rv = self._rest(name, password, "getCoverArt", id=tid)
            self.assertEqual(rv.mimetype, "image/png")
            self.assertTrue(rv.data.startswith(b"\x89PNG"))

            self.client.post("/api/logout")
            self._login(name, password)
            rv = self.client.get(f"/api/services/list/album/{aid}?fmt=csv")
            self.assertEqual(rv.status_code, 200)
            self.assertIn("[silence]", rv.get_data(as_text=True))
            self.client.post("/api/logout")

    def test_a_shared_track_stays_readable(self):
        rv = self._rest("mallory", "M4llory", "getSong", id=str(self.shared.id))
        body = rv.get_json()["subsonic-response"]
        self.assertEqual(body["status"], "ok")
        self.assertEqual(body["song"]["title"], "Shared song")


class AccessSnapshotFailClosedTestCase(unittest.TestCase):
    """S-07: a failed read of the grants never opens the server's account."""

    def setUp(self):
        from tests.testbase import TestBase
        from neezer import access

        self._base = TestBase()
        self._base.setUp()
        self.alice = User.get(name="alice")
        self.bob = User.get(name="bob")
        access.invalidate()
        ProviderGrant.delete().execute()
        self.row = access.ensure_server_account(owner=self.alice)
        access.set_grants(
            "deezer", str(self.row.id), "*", [f"u:{self.alice.id}"]
        )
        access.invalidate()
        self.access = access

    def tearDown(self):
        self.access.invalidate()
        self._base.tearDown()

    def test_a_failed_load_keeps_the_last_grants(self):
        self.assertTrue(self.access.can(self.alice, "deezer", "stream"))
        self.assertFalse(self.access.can(self.bob, "deezer", "stream"))
        last = self.access.snapshot()
        with mock.patch.object(self.access, "CACHE_SECONDS", 0), \
                mock.patch.object(self.access, "_meta_get", return_value="changed"), \
                mock.patch.object(self.access, "_load", side_effect=RuntimeError("boom")):
            self.assertIs(self.access.snapshot(), last)
            self.assertFalse(self.access.can(self.bob, "deezer", "stream"))
            self.assertTrue(self.access.can(self.alice, "deezer", "stream"))

    def test_a_failed_first_load_opens_nothing(self):
        self.access.invalidate()
        with mock.patch.object(self.access, "_booted", set()), \
                mock.patch.object(self.access, "_meta_get", return_value="changed"), \
                mock.patch.object(self.access, "_load", side_effect=RuntimeError("boom")):
            snap = self.access.snapshot()
        self.assertFalse(snap.allowed(str(self.bob.id), "deezer", "stream"))
        self.assertFalse(snap.allowed(str(self.alice.id), "deezer", "stream"))
        self.assertFalse(snap.has_server_account())
        self.assertEqual(snap.grants, {})


class ArlStorageTestCase(unittest.TestCase):
    """S-03: the server's ARL is an encrypted account secret, not a Meta value."""

    def setUp(self):
        from tests.testbase import TestBase

        self._base = TestBase()
        self._base.setUp()
        self.bob = User.get(name="bob")

    def tearDown(self):
        self._base.tearDown()

    def test_saving_the_arl_makes_the_saver_the_holder(self):
        from neezer import access, deezer as dzmod
        from neezer.managers.user import encrypt_password

        ProviderAccount.create(
            provider="deezer",
            user=self.bob.id,
            label="Bob",
            secret=encrypt_password("p" * 64),
            mode="arl",
            status="ok",
        )
        dzmod.store_arl("s" * 64)
        row = access.ensure_server_account(owner=self.bob)
        self.assertEqual(row.user_id, self.bob.id)
        self.assertEqual(row.mode, "server")
        self.assertTrue(row.secret.startswith("server-arl:gcm:"))
        self.assertEqual(dzmod.stored_arl(), "s" * 64)
        self.assertEqual(
            ProviderAccount.select().where(ProviderAccount.provider == "deezer").count(), 1
        )


class ArlUpgradeTestCase(unittest.TestCase):
    """S-03 upgrade path: an old database carrying Meta.deezer_arl boots with
    the same account, its secret now encrypted."""

    def setUp(self):
        from tests.testbase import TestConfig

        self.__db = tempfile.mkstemp()
        self.__dir = tempfile.mkdtemp()
        self.archive = tempfile.mkdtemp()
        self.cfg = TestConfig(True, True)
        self.cfg.BASE["database_uri"] = "sqlite:///" + self.__db[1]
        self.cfg.WEBAPP["cache_dir"] = self.__dir
        self.cfg.DEEZER["archive_dir"] = self.archive
        self.cfg.DEEZER["enabled"] = False
        self.cfg.DEEZER["arl"] = None

    def tearDown(self):
        release_database()
        for d in (self.__dir, self.archive):
            shutil.rmtree(d, ignore_errors=True)
        os.close(self.__db[0])
        os.remove(self.__db[1])

    def test_an_old_plaintext_arl_is_migrated_at_boot(self):
        from neezer import deezer as dzmod

        arl = "c" * 192
        create_application(self.cfg)
        Meta.create(key="deezer_arl", value=arl)
        release_database()

        app = create_application(self.cfg)
        self.assertIsNotNone(app.deezer)
        self.assertEqual(app.deezer.arl, arl)
        self.assertIsNone(Meta.get_or_none(Meta.key == "deezer_arl"))
        row = ProviderAccount.get(
            ProviderAccount.provider == "deezer", ProviderAccount.mode == "server"
        )
        self.assertTrue(row.secret.startswith("server-arl:gcm:"))
        self.assertNotIn(arl, row.secret)
        self.assertEqual(dzmod.stored_arl(), arl)


if __name__ == "__main__":
    unittest.main()
