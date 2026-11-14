# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Collaborative playlists, EDGES slice: Subsonic, importers, readers, limits.

What the plan makes this slice responsible for, and what each class pins:

* ``SubsonicMatrixTestCase`` — a member reads whatever their role can read, the
  owner/admin writes, a non-member keeps error 50, a public playlist keeps its
  Subsonic meaning, and every count/entry is the caller's VISIBLE view.
* ``ImporterEdgesTestCase`` — the Deezer and personal-library importers write
  the owner member row with the playlist, bump the version when they merge, and
  never touch another account's playlist.
* ``ReaderVisibilityTestCase`` — ZIP export, list-file export and the public
  share link resolve through ``playlist_access``: 404 for a non-member, 403 for
  a member without ``can_manage``, private uploads absent, no provider call.
* ``RemotePolicyTestCase`` — the six new endpoints are classified.
* ``WindowLimiterTestCase`` — the sliding window itself, and the two routes'
  measured thresholds (60 creations/hour, 30 membership writes/10 min).
"""

import io
import os
import threading
import unittest
import uuid
import zipfile
from unittest import mock

from neezer import libraryimport
from neezer.db import (
    Playlist,
    PlaylistMember,
    PlaylistTrack,
    User,
)
from neezer.deezer import ids
from neezer.deezer.importer import DeezerImporter
from neezer.managers.user import UserManager
from neezer.ratelimit import WindowLimiter
from neezer.webui import remote as R

from .test_deezer import count_statements
from .test_detach_a import _AppCase


class _EdgesCase(_AppCase):
    """One server, four accounts: alice (admin), bob, chloe, dave, erin."""

    def setUp(self):
        super().setUp()
        from neezer.webui import _reset_playlist_limits
        from neezer.webui.playlist_members import _reset_for_tests

        _reset_playlist_limits()
        _reset_for_tests()
        for name, password in (
            ("bob", "B0bb"),
            ("chloe", "Chl03"),
            ("dave", "Dav3"),
            ("erin", "Er1n"),
        ):
            UserManager.add(name, password)
        self.alice = User.get(User.name == "alice")
        self.bob = User.get(User.name == "bob")
        self.chloe = User.get(User.name == "chloe")
        self.dave = User.get(User.name == "dave")
        self.erin = User.get(User.name == "erin")
        self.bob_client = self._client("bob", "B0bb")
        self.chloe_client = self._client("chloe", "Chl03")
        self.dave_client = self._client("dave", "Dav3")
        self.erin_client = self._client("erin", "Er1n")
        self.app.deezer_prefetch = None

    def tearDown(self):
        from neezer.webui import _reset_playlist_limits
        from neezer.webui.playlist_members import _reset_for_tests

        _reset_playlist_limits()
        _reset_for_tests()
        super().tearDown()

    def _client(self, name, password):
        client = self.app.test_client()
        rv = client.post("/api/login", json={"username": name, "password": password})
        self.assertEqual(rv.status_code, 200, rv.get_json())
        return client

    def _playlist(self, owner, name="Route 66", public=False):
        return Playlist.create(user=owner, name=name, public=public)

    def _join(self, playlist, user, role):
        return PlaylistMember.create(playlist=playlist, user=user, role=role)

    def _track(self, sng_id, title=None, duration=200, album="600"):
        from neezer.deezer import library

        root = library.get_root_folder(self.archive)
        return library.upsert_track(
            {
                "SNG_ID": str(sng_id),
                "SNG_TITLE": title or f"Song {sng_id}",
                "ART_NAME": f"Artist {album}",
                "ART_ID": f"9{album}",
                "ALB_TITLE": f"Album {album}",
                "ALB_ID": str(album),
                "DURATION": str(duration),
                "TRACK_NUMBER": "1",
                "DISK_NUMBER": "1",
            },
            root,
        )

    def _archived(self, sng_id, title=None):
        """A Deezer row whose bytes are on disk, as archiving leaves it."""
        track = self._track(sng_id, title)
        os.makedirs(os.path.dirname(track.path), exist_ok=True)
        with open(track.path, "wb") as fh:
            fh.write(b"flac-bytes-" + str(sng_id).encode() * 40)
        track.last_modification = 1700000000
        track.save()
        return track

    def _upload(self, title, owner):
        """A real private upload: a file on disk plus its Track row."""
        from neezer.deezer import library, local

        root = library.get_root_folder(self.archive)
        folder = os.path.join(self.archive, "Uploads", owner.name)
        os.makedirs(folder, exist_ok=True)
        path = os.path.join(folder, title.replace(" ", "_") + ".mp3")
        with open(path, "wb") as fh:
            fh.write(b"upload-bytes")

        class FakeTag:
            artist = "Local Band"
            albumartist = None
            album = "Demo"
            genre = "Indie"
            disc = 1
            track = 1
            year = None
            length = 180.0
            bitrate = 320000
            images = []

        FakeTag.title = title
        original = local._load_tag
        local._load_tag = lambda p: FakeTag() if p == path else None
        try:
            return local.import_local_file(path, root, owner=owner)
        finally:
            local._load_tag = original

    def _fill(self, playlist, tracks):
        with self.app.app_context():
            for index, track in enumerate(tracks):
                PlaylistTrack.create(playlist=playlist, track=track, index=index)
        return playlist

    def _rest(self, endpoint, user="bob", password="B0bb", **params):
        params.update(
            {"u": user, "p": password, "c": "edges", "v": "1.12.0", "f": "json"}
        )
        rv = self.app.test_client().get(
            f"/rest/{endpoint}", query_string=params
        )
        self.assertEqual(rv.status_code, 200, rv.data)
        return rv.get_json()["subsonic-response"]


class SubsonicMatrixTestCase(_EdgesCase):
    """Subsonic reads follow the role; writes stay owner/admin; error 50 keeps
    every old client's behaviour."""

    def setUp(self):
        super().setUp()
        self.shared1 = self._track("1", "One", duration=200)
        self.private = self._track("2", "Two", duration=100)
        self.private.owner = self.bob
        self.private.save()
        self.shared3 = self._track("3", "Three", duration=300)
        self.pl = self._playlist(self.bob)
        self._fill(self.pl, [self.shared1, self.private, self.shared3])
        self._join(self.pl, self.chloe, "editor")
        self._join(self.pl, self.dave, "viewer")

    def test_a_member_reads_the_visible_list_and_a_stranger_gets_error_50(self):
        for name, password in (("chloe", "Chl03"), ("dave", "Dav3")):
            data = self._rest("getPlaylist", name, password, id=str(self.pl.id))
            self.assertEqual(data["status"], "ok", name)
            playlist = data["playlist"]
            self.assertEqual(
                [entry["title"] for entry in playlist["entry"]], ["One", "Three"]
            )
            self.assertEqual(playlist["songCount"], 2)
            self.assertEqual(playlist["duration"], 500)
            self.assertEqual(playlist["name"], "[bob] Route 66")
            self.assertEqual(playlist["owner"], "bob")

        self.assertEqual(
            self._rest("getPlaylist", "erin", "Er1n", id=str(self.pl.id))["error"][
                "code"
            ],
            50,
        )

        admin = self._rest("getPlaylist", "alice", "Alic3", id=str(self.pl.id))
        self.assertEqual([e["title"] for e in admin["playlist"]["entry"]],
                         ["One", "Two", "Three"])
        self.assertEqual(admin["playlist"]["songCount"], 3)
        self.assertEqual(admin["playlist"]["duration"], 600)

    def test_the_listing_follows_membership_not_admin_and_hides_private_uploads(self):
        data = self._rest("getPlaylists", "dave", "Dav3")
        listed = {entry["id"]: entry for entry in data["playlists"]["playlist"]}
        self.assertIn(str(self.pl.id), listed)
        self.assertEqual(listed[str(self.pl.id)]["songCount"], 2)
        self.assertEqual(listed[str(self.pl.id)]["duration"], 500)

        stranger = self._rest("getPlaylists", "erin", "Er1n")
        self.assertEqual(
            stranger.get("playlists", {}).get("playlist", []), []
        )

    def test_a_public_playlist_keeps_its_subsonic_meaning(self):
        self.pl.public = True
        self.pl.save()
        data = self._rest("getPlaylist", "erin", "Er1n", id=str(self.pl.id))
        self.assertEqual(data["status"], "ok")
        self.assertEqual(
            [entry["title"] for entry in data["playlist"]["entry"]], ["One", "Three"]
        )
        self.assertEqual(data["playlist"]["songCount"], 2)
        self.assertTrue(data["playlist"]["public"])
        listed = self._rest("getPlaylists", "erin", "Er1n")
        self.assertIn(
            str(self.pl.id),
            {entry["id"] for entry in listed["playlists"]["playlist"]},
        )
        self.assertEqual(
            self._rest(
                "updatePlaylist", "erin", "Er1n", playlistId=str(self.pl.id), name="x"
            )["error"]["code"],
            50,
        )

    def test_writes_are_owner_or_admin_never_editor_or_viewer(self):
        for name, password in (("chloe", "Chl03"), ("dave", "Dav3")):
            for endpoint, params in (
                ("updatePlaylist", {"playlistId": str(self.pl.id), "name": "Nope"}),
                ("deletePlaylist", {"id": str(self.pl.id)}),
                ("createPlaylist", {"playlistId": str(self.pl.id)}),
            ):
                self.assertEqual(
                    self._rest(endpoint, name, password, **params)["error"]["code"],
                    50,
                    f"{name} {endpoint}",
                )
        self.assertEqual(Playlist[self.pl.id].name, "Route 66")
        self.assertEqual(Playlist[self.pl.id].version, 1)
        self.assertEqual(
            PlaylistTrack.select().where(PlaylistTrack.playlist == self.pl.id).count(),
            3,
        )

        self.assertEqual(
            self._rest(
                "updatePlaylist", "bob", "B0bb", playlistId=str(self.pl.id), name="Renamed"
            )["status"],
            "ok",
        )
        fresh = Playlist[self.pl.id]
        self.assertEqual((fresh.name, fresh.version, fresh.last_edited_by),
                         ("Renamed", 2, "bob"))

        self.assertEqual(
            self._rest(
                "updatePlaylist", "alice", "Alic3", playlistId=str(self.pl.id),
                comment="support",
            )["status"],
            "ok",
        )
        fresh = Playlist[self.pl.id]
        self.assertEqual((fresh.comment, fresh.version, fresh.last_edited_by),
                         ("support", 3, "alice"))

    def test_creation_and_overwrite_write_the_owner_row_and_the_counters(self):
        self.assertEqual(
            self._rest(
                "createPlaylist", "bob", "B0bb", name="Mine", songId=str(self.shared1.id)
            )["status"],
            "ok",
        )
        mine = Playlist.get(Playlist.name == "Mine")
        self.assertEqual(mine.user_id, self.bob.id)
        self.assertEqual((mine.version, mine.members_version), (1, 1))
        rows = list(PlaylistMember.select().where(PlaylistMember.playlist == mine))
        self.assertEqual([(row.user_id, row.role) for row in rows],
                         [(self.bob.id, "owner")])
        self.assertEqual(rows[0].added, mine.created)
        self.assertEqual(mine.get_tracks()[0].id, self.shared1.id)

        self.assertEqual(
            self._rest(
                "createPlaylist", "bob", "B0bb", playlistId=str(mine.id),
                songId=str(self.shared3.id),
            )["status"],
            "ok",
        )
        mine = Playlist[mine.id]
        self.assertEqual(mine.version, 2)
        self.assertEqual([t.id for t in mine.get_tracks()], [self.shared3.id])
        self.assertEqual(
            PlaylistMember.select().where(PlaylistMember.playlist == mine.id).count(), 1
        )

        self.assertEqual(
            self._rest("deletePlaylist", "bob", "B0bb", id=str(mine.id))["status"], "ok"
        )
        self.assertIsNone(Playlist.get_or_none(Playlist.id == mine.id))

    def test_a_private_upload_is_never_listed_nor_addable(self):
        secret = self._upload("Their Secret", self.dave)
        self.assertEqual(
            self._rest(
                "updatePlaylist", "bob", "B0bb", playlistId=str(self.pl.id),
                songIdToAdd=str(secret.id),
            )["error"]["code"],
            70,
        )
        self.assertNotIn(
            secret.id,
            [t.id for t in Playlist[self.pl.id].get_tracks()],
        )
        data = self._rest("getPlaylist", "alice", "Alic3", id=str(self.pl.id))
        self.assertEqual(data["playlist"]["songCount"], 3)

    def test_removing_by_index_follows_the_callers_visible_list(self):
        hidden = self._upload("Chloe Only", self.chloe)
        PlaylistTrack.update(track=hidden.id).where(
            (PlaylistTrack.playlist == self.pl.id)
            & (PlaylistTrack.track == self.private.id)
        ).execute()

        seen = self._rest("getPlaylist", "bob", "B0bb", id=str(self.pl.id))
        self.assertEqual([e["title"] for e in seen["playlist"]["entry"]],
                         ["One", "Three"])

        self.assertEqual(
            self._rest(
                "updatePlaylist", "bob", "B0bb", playlistId=str(self.pl.id),
                songIndexToRemove="1",
            )["status"],
            "ok",
        )
        titles = [
            t.title for t in Playlist[self.pl.id].get_tracks()
        ]
        self.assertEqual(titles, ["One", "Chloe Only"])
        self.assertEqual(Playlist[self.pl.id].version, 2)

    def test_a_member_listing_costs_one_statement_per_playlist_not_two(self):
        """61 playlists: 66 statements (1 auth, 1 ClientPrefs, 1 list, 1 grouped
        count, 61 per-playlist aggregates, 1 INSERT). Before the owner join it
        was 127: ``Playlist.user`` was fetched once per row."""
        for i in range(60):
            self._playlist(self.dave, name=f"P{i:02d}")
        with count_statements() as stmts:
            data = self._rest("getPlaylists", "dave", "Dav3")
        self.assertEqual(len(data["playlists"]["playlist"]), 61)
        self.assertEqual(sum(stmts.values()), 66, dict(stmts))


class ImporterEdgesTestCase(_EdgesCase):
    """The importers own what they import and nothing else."""

    def _raw(self, sng_id, title):
        return {
            "SNG_ID": str(sng_id),
            "SNG_TITLE": title,
            "ART_NAME": "Artist",
            "ART_ID": "900",
            "ALB_TITLE": "Album",
            "ALB_ID": "600",
            "DURATION": "200",
            "TRACK_NUMBER": "1",
            "DISK_NUMBER": "1",
        }

    def test_a_new_import_writes_the_owner_row_and_a_merge_bumps_the_version(self):
        importer = DeezerImporter(self.app.deezer, "bob")
        local_id = ids.playlist_uuid("123")
        playlist, count = importer._materialize_playlist(
            local_id, "Mix", "123", [self._raw(1, "One")], merge=True
        )
        self.assertEqual((playlist.user_id, playlist.version, count),
                         (self.bob.id, 1, 1))
        rows = list(PlaylistMember.select().where(PlaylistMember.playlist == playlist))
        self.assertEqual([(r.user_id, r.role) for r in rows], [(self.bob.id, "owner")])

        merged, count = importer._materialize_playlist(
            local_id, "Mix", "123", [self._raw(1, "One"), self._raw(2, "Two")],
            merge=True,
        )
        self.assertEqual((merged.version, count), (2, 2))
        self.assertEqual(
            PlaylistMember.select().where(PlaylistMember.playlist == playlist).count(),
            1,
        )

        again, count = importer._materialize_playlist(
            local_id, "Mix", "123", [self._raw(1, "One"), self._raw(2, "Two")],
            merge=True,
        )
        self.assertEqual((again.version, count), (2, 2))

    def test_another_accounts_playlist_is_never_touched_or_adopted(self):
        local_id = ids.playlist_uuid("123")
        theirs = Playlist.create(
            id=local_id, user=self.bob, name="Bob's", deezer_id="123"
        )
        PlaylistTrack.create(playlist=theirs, track=self._track("9"), index=0)
        PlaylistMember.create(playlist=theirs, user=self.chloe, role="editor")

        importer = DeezerImporter(self.app.deezer, "alice")
        mine, _ = importer._materialize_playlist(
            local_id, "Mix", "123", [self._raw(1, "One")], merge=True
        )

        self.assertNotEqual(mine.id, local_id)
        self.assertEqual(mine.id, uuid.uuid5(local_id, str(self.alice.id)))
        self.assertEqual((mine.user_id, mine.deezer_id), (self.alice.id, "123"))
        self.assertEqual(
            [r.user_id for r in PlaylistMember.select().where(
                PlaylistMember.playlist == mine
            )],
            [self.alice.id],
        )

        theirs = Playlist[local_id]
        self.assertEqual(theirs.name, "Bob's")
        self.assertEqual([t.deezer_id for t in theirs.get_tracks()], ["9"])
        self.assertEqual(
            [r.user_id for r in PlaylistMember.select().where(
                PlaylistMember.playlist == theirs
            )],
            [self.chloe.id],
        )

    def test_a_numeric_deezer_id_resolves_to_the_callers_own_copy(self):
        mine = Playlist.create(user=self.alice, name="Mine", deezer_id="123")
        Playlist.create(user=self.bob, name="Theirs", deezer_id="123")
        rv = self.client.get("/api/playlist/123")
        self.assertEqual(rv.status_code, 200, rv.get_json())
        self.assertEqual(rv.get_json()["playlist"]["id"], str(mine.id))

    def test_the_personal_import_writes_the_owner_row_in_the_same_transaction(self):
        first, second = self._track("1"), self._track("2")
        playlist = libraryimport._save_playlist(self.bob, uuid.uuid4(), "Mine", None, [first])
        self.assertEqual((playlist.user_id, playlist.version), (self.bob.id, 1))
        rows = list(PlaylistMember.select().where(PlaylistMember.playlist == playlist))
        self.assertEqual([(r.user_id, r.role) for r in rows], [(self.bob.id, "owner")])
        self.assertEqual(rows[0].added, playlist.created)

        libraryimport._save_playlist(self.bob, playlist.id, "Ignored", None, [first, second])
        playlist = Playlist[playlist.id]
        self.assertEqual(playlist.version, 2)
        self.assertEqual(len(playlist.get_tracks()), 2)
        self.assertEqual(
            PlaylistMember.select().where(PlaylistMember.playlist == playlist.id).count(),
            1,
        )

    def test_the_personal_import_never_adopts_another_accounts_playlist(self):
        local_id = uuid.uuid4()
        theirs = Playlist.create(id=local_id, user=self.bob, name="Bob's")
        PlaylistMember.create(playlist=theirs, user=self.chloe, role="viewer")
        mine = libraryimport._save_playlist(
            self.alice, local_id, "Alice's", None, [self._track("3")]
        )
        self.assertNotEqual(mine.id, local_id)
        self.assertEqual(mine.user_id, self.alice.id)
        self.assertEqual(Playlist[local_id].name, "Bob's")
        self.assertEqual(
            PlaylistMember.select().where(PlaylistMember.playlist == theirs).count(), 1
        )
        self.assertEqual(
            [r.user_id for r in PlaylistMember.select().where(
                PlaylistMember.playlist == mine
            )],
            [self.alice.id],
        )


class ReaderVisibilityTestCase(_EdgesCase):
    """Export, list file and the public link: members only, visible rows only."""

    def setUp(self):
        super().setUp()
        self.one = self._archived("1", "One")
        self.secret = self._archived("2", "Two")
        self.secret.owner = self.chloe
        self.secret.save()
        self.three = self._archived("3", "Three")
        self.pl = self._playlist(self.bob)
        self._fill(self.pl, [self.one, self.secret, self.three])
        self._join(self.pl, self.chloe, "editor")
        self._join(self.pl, self.dave, "viewer")

    def test_the_zip_export_is_the_callers_visible_list(self):
        rv = self.dave_client.get(f"/api/export/playlist/{self.pl.id}?fmt=flac")
        self.assertEqual(rv.status_code, 200)
        archive = zipfile.ZipFile(io.BytesIO(rv.data))
        names = archive.namelist()
        self.assertIn("001 - Artist 600 - One.flac", names)
        self.assertIn("002 - Artist 600 - Three.flac", names)
        self.assertEqual(len([n for n in names if n.endswith(".flac")]), 2)
        self.assertNotIn(
            "Artist 600 - Two.flac", " ".join(names)
        )
        self.assertEqual(
            self.erin_client.get(f"/api/export/playlist/{self.pl.id}?fmt=flac").status_code,
            404,
        )
        self.assertEqual(
            self.erin_client.get(f"/api/export/playlist/{self.pl.id}?fmt=flac").get_json(),
            {"error": "not found"},
        )

    def test_the_list_file_is_the_callers_visible_list(self):
        owner_upload = self._upload("Owner Upload", self.bob)
        self.pl.add(owner_upload)
        rv = self.dave_client.get(f"/api/services/list/playlist/{self.pl.id}?fmt=txt")
        self.assertEqual(rv.status_code, 200)
        body = rv.data.decode()
        self.assertIn("Artist 600 - One", body)
        self.assertIn("Artist 600 - Three", body)
        self.assertNotIn("Two", body)
        self.assertNotIn("Owner Upload", body)
        own = self.bob_client.get(f"/api/services/list/playlist/{self.pl.id}?fmt=txt")
        self.assertIn("Owner Upload", own.data.decode())
        self.assertEqual(
            self.erin_client.get(
                f"/api/services/list/playlist/{self.pl.id}?fmt=csv"
            ).status_code,
            404,
        )

    def test_the_share_link_is_owner_only_and_never_names_anybody(self):
        for method in ("get", "post", "delete"):
            rv = getattr(self.erin_client, method)(
                f"/api/playlist/{self.pl.id}/share", json={} if method == "post" else None
            )
            self.assertEqual(rv.status_code, 404, method)
        for method in ("get", "post", "delete"):
            rv = getattr(self.chloe_client, method)(
                f"/api/playlist/{self.pl.id}/share", json={} if method == "post" else None
            )
            self.assertEqual(rv.status_code, 403, method)
            self.assertEqual(rv.get_json(), {"error": "forbidden"})

        created = self.bob_client.post(
            f"/api/playlist/{self.pl.id}/share", json={}
        )
        self.assertEqual(created.status_code, 200)
        token = created.get_json()["link"]["token"]

        public = self.app.test_client().get(f"/api/shared/playlist/{token}")
        self.assertEqual(public.status_code, 200)
        payload = public.get_json()
        self.assertEqual(payload["playlist"]["owner"], "bob")
        self.assertNotIn("members", payload["playlist"])
        self.assertNotIn("last_edited_by", payload["playlist"])
        self.assertNotIn("last_edited_at", payload["playlist"])
        self.assertNotIn("version", payload["playlist"])
        self.assertEqual(
            [track["title"] for track in payload["tracks"]], ["One", "Three"]
        )
        self.assertEqual(payload["playlist"]["nb_tracks"], 2)

    def test_the_unavailable_list_names_only_the_playlists_the_caller_may_open(self):
        from neezer.db import now

        self.one.unavailable = now()
        self.one.save()
        private = self._playlist(self.bob, name="Private Nook")
        PlaylistTrack.create(playlist=private, track=self.one, index=0)

        def listed(client):
            rows = client.get("/api/unavailable").get_json()["tracks"]
            entry = next(r for r in rows if r["title"] == "One")
            return [p["title"] for p in entry["playlists"]]

        self.assertEqual(listed(self.dave_client), ["Route 66"])
        self.assertEqual(listed(self.bob_client), ["Private Nook", "Route 66"])
        self.assertEqual(listed(self.erin_client), [])
        self.assertEqual(len(listed(self.client)), 2)

    def test_no_reader_touches_the_provider(self):
        spy = mock.MagicMock()
        self.app.deezer = spy
        self.dave_client.get(f"/api/export/playlist/{self.pl.id}?fmt=flac")
        self.dave_client.get(f"/api/services/list/playlist/{self.pl.id}?fmt=csv")
        link = self.bob_client.post(f"/api/playlist/{self.pl.id}/share", json={})
        token = link.get_json()["link"]["token"]
        self.app.test_client().get(f"/api/shared/playlist/{token}")
        self.bob_client.delete(f"/api/playlist/{self.pl.id}/share")
        self.assertEqual(spy.mock_calls, [])


class LegacyFrontendTestCase(_EdgesCase):
    """The server-rendered playlist page follows the same read matrix."""

    def _frontend(self, name, password):
        from neezer.frontend import CSRF_SESSION_KEY

        client = self.app.test_client()
        with client.session_transaction() as session:
            session[CSRF_SESSION_KEY] = "test-token"
        rv = client.post(
            "/user/login",
            data={"user": name, "password": password, "_csrf": "test-token"},
        )
        self.assertEqual(rv.status_code, 302, rv.data)
        with client.session_transaction() as session:
            session[CSRF_SESSION_KEY] = "test-token"
        return client

    def test_a_member_sees_visible_rows_a_stranger_does_not_and_only_the_owner_writes(self):
        playlist = self._playlist(self.bob, name="Legacy")
        one = self._track("1", "One")
        hidden = self._upload("Secret Upload", self.chloe)
        PlaylistTrack.create(playlist=playlist, track=one, index=0)
        PlaylistTrack.create(playlist=playlist, track=hidden, index=1)
        self._join(playlist, self.dave, "viewer")
        self._join(playlist, self.chloe, "editor")

        viewer = self._frontend("dave", "Dav3")
        page = viewer.get(f"/playlist/{playlist.id}")
        self.assertEqual(page.status_code, 200)
        body = page.get_data(as_text=True)
        self.assertIn("One", body)
        self.assertNotIn("Secret Upload", body)
        export = viewer.get(f"/playlist/{playlist.id}/export").get_data(as_text=True)
        self.assertIn("One", export)
        self.assertNotIn("Secret_Upload", export)

        editor = self._frontend("chloe", "Chl03")
        own = editor.get(f"/playlist/{playlist.id}").get_data(as_text=True)
        self.assertIn("Secret Upload", own)
        refused = editor.post(
            f"/playlist/{playlist.id}",
            data={"name": "Hacked", "_csrf": "test-token"},
            follow_redirects=True,
        )
        self.assertIn("not allowed", refused.get_data(as_text=True))
        self.assertEqual(Playlist[playlist.id].name, "Legacy")

        stranger = self._frontend("erin", "Er1n")
        self.assertEqual(stranger.get(f"/playlist/{playlist.id}").status_code, 302)


class RemotePolicyTestCase(unittest.TestCase):
    """A lent session reads who is on a playlist, never changes it."""

    def test_the_two_reads_are_read_and_the_four_mutations_are_never(self):
        self.assertEqual(R.POLICY["playlist_version"], "read")
        self.assertEqual(R.POLICY["playlist_members"], "read")
        for name in (
            "playlist_invite",
            "playlist_member_role",
            "playlist_member_remove",
            "playlist_leave",
        ):
            self.assertEqual(R.POLICY[name], R.NEVER, name)
            self.assertFalse(R.allows("admin", None), name)
        for name in (
            "create_playlist",
            "edit_playlist",
            "delete_playlist",
            "add_playlist_tracks",
            "remove_playlist_tracks",
            "reorder_playlist",
        ):
            self.assertEqual(R.POLICY[name], "full", name)
            self.assertTrue(R.allows("full", R.POLICY[name]), name)
            self.assertFalse(R.allows("read", R.POLICY[name]), name)


class WindowLimiterTestCase(unittest.TestCase):
    """The success-side window: bounded, thread-safe, resettable."""

    def test_it_counts_every_call_and_answers_the_wait(self):
        limiter = WindowLimiter(3, 60)
        self.assertEqual([limiter.allow("a")[0] for _ in range(3)], [True] * 3)
        allowed, retry = limiter.allow("a")
        self.assertFalse(allowed)
        self.assertGreaterEqual(retry, 1)
        self.assertLessEqual(retry, 61)
        self.assertTrue(limiter.allow("b")[0], "another key has its own window")

    def test_the_key_table_is_bounded(self):
        limiter = WindowLimiter(1, 60, max_keys=2)
        for key in ("a", "b", "c", "d", "e"):
            self.assertTrue(limiter.allow(key)[0])
        self.assertLessEqual(len(limiter), 2)

    def test_a_concurrent_burst_admits_exactly_the_cap(self):
        limiter = WindowLimiter(100, 60)
        results = []
        lock = threading.Lock()

        def burst():
            for _ in range(20):
                allowed, _retry = limiter.allow("same")
                with lock:
                    results.append(allowed)

        threads = [threading.Thread(target=burst) for _ in range(8)]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join()
        self.assertEqual(results.count(True), 100)

    def test_reset_clears_one_key_and_clear_clears_all(self):
        limiter = WindowLimiter(1, 60)
        self.assertTrue(limiter.allow("a")[0])
        self.assertFalse(limiter.allow("a")[0])
        limiter.reset("a")
        self.assertTrue(limiter.allow("a")[0])
        limiter.allow("b")
        limiter.clear()
        self.assertEqual(len(limiter), 0)
        self.assertTrue(limiter.allow("a")[0])


class RateLimitRoutesTestCase(_EdgesCase):
    """The two measured thresholds the plan names, on the wire."""

    def test_sixty_creations_an_hour_then_429(self):
        for i in range(60):
            rv = self.bob_client.post("/api/playlists", json={"title": f"P{i:02d}"})
            self.assertEqual(rv.status_code, 200, rv.get_json())
        rv = self.bob_client.post("/api/playlists", json={"title": "one too many"})
        self.assertEqual(rv.status_code, 429)
        self.assertGreaterEqual(int(rv.headers["Retry-After"]), 1)
        self.assertEqual(Playlist.select().where(Playlist.user == self.bob).count(), 60)
        self.assertEqual(self.chloe_client.post("/api/playlists", json={"title": "x"}).status_code, 200)

    def test_thirty_membership_writes_in_ten_minutes_then_429(self):
        pl = self._playlist(self.bob)
        for _ in range(30):
            rv = self.bob_client.post(
                f"/api/playlist/{pl.id}/members",
                json={"username": "chloe", "role": "editor"},
            )
            self.assertEqual(rv.status_code, 200, rv.get_json())
        rv = self.bob_client.post(
            f"/api/playlist/{pl.id}/members",
            json={"username": "chloe", "role": "viewer"},
        )
        self.assertEqual(rv.status_code, 429)
        self.assertGreaterEqual(int(rv.headers["Retry-After"]), 1)
        self.assertEqual(
            PlaylistMember.get(
                (PlaylistMember.playlist == pl.id)
                & (PlaylistMember.user == self.chloe.id)
            ).role,
            "editor",
        )
        self.assertEqual(self.chloe_client.post(
            f"/api/playlist/{pl.id}/leave", json={}
        ).status_code, 200)


if __name__ == "__main__":
    unittest.main()
