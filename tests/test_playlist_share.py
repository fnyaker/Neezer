# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""The public read-only playlist link (neezer/webui/playlist_share.py).

The link is a capability: a copy of the table opens nothing, the token carries
an HMAC under the app's secret, a revocation is dead on the very next public
request, a private upload of somebody else is never listed or served, and no
public route ever reaches the provider. What these tests pin, with numbers:
the public page's payload is 5 SQL statements for a playlist of any size (the
rows are read as tuples, credits in one batched query), and a stream answers
with byte ranges straight from the archived file.
"""

import os
import shutil
import tempfile
import unittest
from datetime import timedelta
from unittest import mock

from neezer.config import DefaultConfig
from neezer.db import Playlist, PlaylistShare, User, now, release_database
from neezer.managers.user import UserManager
from neezer.web import create_application

from .test_deezer import count_statements


def _jpeg():
    """A real, tiny JPEG — the only shape ``covers.is_valid_cover`` accepts."""
    import io

    from PIL import Image

    buf = io.BytesIO()
    Image.new("RGB", (2, 2), "white").save(buf, "JPEG")
    return buf.getvalue()


class PlaylistShareTestCase(unittest.TestCase):
    def setUp(self):
        self.__db = tempfile.mkstemp()
        self.__dir = tempfile.mkdtemp()
        self.archive = tempfile.mkdtemp()

        db_path = self.__db[1]
        cache = self.__dir

        class Config(DefaultConfig):
            TESTING = True

            def __init__(self):
                super().__init__()
                self.BASE = dict(self.BASE, database_uri="sqlite:///" + db_path)
                self.WEBAPP = dict(self.WEBAPP, cache_dir=cache, mount_webui=True, mount_api=True)

        self.app = create_application(Config())
        UserManager.add("alice", "Alic3", admin=True)
        UserManager.add("bob", "B0bbb")
        self.alice = User.get(name="alice")
        self.bob = User.get(name="bob")

        from neezer.webui.playlist_share import _stream_limiter

        _stream_limiter.clear()

        self.owner = self.app.test_client()
        self.owner.post("/api/login", json={"username": "alice", "password": "Alic3"})
        self.other = self.app.test_client()
        self.other.post("/api/login", json={"username": "bob", "password": "B0bbb"})

    def tearDown(self):
        release_database()
        shutil.rmtree(self.__dir, ignore_errors=True)
        shutil.rmtree(self.archive, ignore_errors=True)
        os.close(self.__db[0])
        os.remove(self.__db[1])

    # -- fixtures ------------------------------------------------------------

    def _deezer_track(self, sng_id, title="Deezer Song", archived=True):
        """A DB row for a Deezer track; when `archived`, a file is on disk at
        its archive path (the shape archive._finalize_archive stamps)."""
        from neezer.deezer import library

        root = library.get_root_folder(self.archive)
        t = library.upsert_track(
            {
                "SNG_ID": sng_id,
                "SNG_TITLE": title,
                "ART_NAME": "Deezer Artist",
                "ART_ID": "400",
                "ALB_TITLE": "Deezer Album",
                "ALB_ID": "410",
                "DURATION": "200",
                "TRACK_NUMBER": "1",
                "DISK_NUMBER": "1",
            },
            root,
        )
        if archived:
            os.makedirs(os.path.dirname(t.path), exist_ok=True)
            with open(t.path, "wb") as fh:
                fh.write(b"flac-bytes-" + str(sng_id).encode() * 48)
            t.last_modification = 1700000000
            t.save()
        return t

    def _cover(self, track):
        """The cover the route serves for this track, pre-populated in the cover
        cache exactly as archiving would have put it there."""
        self.app.cache.set(f"localcover-{track.id}", _jpeg())

    def _local_track(self, title, owner=None):
        """A local (uploaded or scanned) track; `owner` marks a private upload."""
        from neezer.deezer import library, local

        root = library.get_root_folder(self.archive)
        folder = os.path.join(self.archive, title.replace(" ", "_"))
        os.makedirs(folder, exist_ok=True)
        path = os.path.join(folder, "song.mp3")
        with open(path, "wb") as fh:
            fh.write(b"local-audio-bytes")

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
        orig = local._load_tag
        local._load_tag = lambda p: FakeTag() if p == path else None
        try:
            return local.import_local_file(path, root, owner=owner)
        finally:
            local._load_tag = orig

    def _playlist(self, owner, name="Ma sélection", comment=None):
        pl = Playlist.create(user=owner, name=name, comment=comment)
        return pl

    def _make_link(self, client, pl, ttl=None):
        body = {} if ttl is None else {"ttl": ttl}
        r = client.post(f"/api/playlist/{pl.id}/share", json=body)
        self.assertEqual(r.status_code, 200, r.get_json())
        return r.get_json()["link"]

    def _public(self):
        """A client with no session at all, as the link's holder."""
        return self.app.test_client()

    # -- the owner's lifecycle --------------------------------------------------

    def test_owner_creates_reads_and_revokes_a_link(self):
        pl = self._playlist(self.alice)
        self.assertEqual(
            self.owner.get(f"/api/playlist/{pl.id}/share").get_json()["link"], None
        )
        link = self._make_link(self.owner, pl)
        rid, mac = link["token"].split(".")
        self.assertEqual(len(mac), 24)  # a token, not the bare row id
        self.assertNotEqual(mac, "0" * 24)
        self.assertEqual(link["url"], f"/pl/{link['token']}")
        self.assertIsNone(link["expires"])
        self.assertIsNotNone(link["created"])
        # One live link per playlist: asking again hands the same one back.
        again = self._make_link(self.owner, pl)
        self.assertEqual(again["token"], link["token"])
        self.assertEqual(PlaylistShare.select().count(), 1)
        # A public request works, then the cut kills it on the very next one.
        self.assertEqual(self._public().get(f"/api/shared/playlist/{link['token']}").status_code, 200)
        self.assertEqual(
            self.owner.delete(f"/api/playlist/{pl.id}/share").get_json(), {"revoked": 1}
        )
        self.assertEqual(
            self._public().get(f"/api/shared/playlist/{link['token']}").status_code, 404
        )
        self.assertEqual(
            self.owner.get(f"/api/playlist/{pl.id}/share").get_json()["link"], None
        )
        # The row stays — the capability is cut, the record remains.
        self.assertEqual(PlaylistShare.select().count(), 1)

    def test_a_ttl_is_checked_and_an_expiry_is_honoured(self):
        pl = self._playlist(self.alice)
        self.assertEqual(
            self.owner.post(f"/api/playlist/{pl.id}/share", json={"ttl": 60}).status_code,
            400,
        )
        self.assertEqual(
            self.owner.post(
                f"/api/playlist/{pl.id}/share", json={"ttl": 31 * 24 * 3600}
            ).status_code,
            400,
        )
        self.assertEqual(
            self.owner.post(f"/api/playlist/{pl.id}/share", json={"ttl": "soon"}).status_code,
            400,
        )
        link = self._make_link(self.owner, pl, ttl=3600)
        self.assertIsNotNone(link["expires"])
        # A link past its expiry is dead, without anyone revoking it.
        row = PlaylistShare.select().first()
        row.expires = now() - timedelta(seconds=1)
        row.save()
        self.assertEqual(
            self._public().get(f"/api/shared/playlist/{link['token']}").status_code, 404
        )
        # And the owner can mint a fresh one once the old one is dead.
        self.assertEqual(self.owner.get(f"/api/playlist/{pl.id}/share").get_json()["link"], None)
        fresh = self._make_link(self.owner, pl)
        self.assertNotEqual(fresh["token"], link["token"])

    def test_only_the_owner_or_an_admin_may_touch_the_link(self):
        UserManager.add("carol", "C4rol")
        carol = self.app.test_client()
        carol.post("/api/login", json={"username": "carol", "password": "C4rol"})
        bobs = self._playlist(self.bob)
        # A stranger — not the owner, not an admin — reads nothing and touches
        # nothing: the playlist reads exactly like one that does not exist.
        self.assertEqual(carol.get(f"/api/playlist/{bobs.id}/share").status_code, 404)
        self.assertEqual(carol.post(f"/api/playlist/{bobs.id}/share", json={}).status_code, 404)
        self.assertEqual(carol.delete(f"/api/playlist/{bobs.id}/share").status_code, 404)
        # The owner sees their own (empty) link state.
        self.assertEqual(self.other.get(f"/api/playlist/{bobs.id}/share").get_json()["link"], None)
        # An admin creates on somebody else's playlist, on the owner's behalf.
        link = self._make_link(self.owner, bobs)
        self.assertEqual(
            PlaylistShare.select().where(PlaylistShare.user == self.alice).count(), 1
        )
        self.assertEqual(
            self._public().get(f"/api/shared/playlist/{link['token']}").status_code, 200
        )

    def test_an_unknown_playlist_is_a_404(self):
        self.assertEqual(
            self.owner.get("/api/playlist/00000000-0000-0000-0000-000000000000/share").status_code,
            404,
        )
        self.assertEqual(
            self.owner.post(
                "/api/playlist/00000000-0000-0000-0000-000000000000/share", json={}
            ).status_code,
            404,
        )

    # -- the public page ---------------------------------------------------------

    def test_a_non_object_body_is_a_400(self):
        pl = self._playlist(self.alice)
        r = self.owner.post(f"/api/playlist/{pl.id}/share", json=[])
        self.assertEqual(r.status_code, 400)
        self.assertEqual(r.get_json(), {"error": "invalid body"})

    def test_the_public_payload_lists_what_the_owner_archived_only(self):
        archived = self._deezer_track("1", "Archived One", archived=True)
        cold = self._deezer_track("2", "Cold One", archived=False)
        mine = self._local_track("My Upload", owner=self.alice)
        theirs = self._local_track("Their Upload", owner=self.bob)
        pl = self._playlist(self.alice, name="On Air", comment="Pour la route")
        for t in (archived, cold, mine, theirs):
            pl.add(t)

        link = self._make_link(self.owner, pl)
        public = self._public()
        r = public.get(f"/api/shared/playlist/{link['token']}")
        self.assertEqual(r.status_code, 200)
        self.assertEqual(r.headers["Cache-Control"], "no-store")
        self.assertEqual(r.headers["X-Robots-Tag"], "noindex")
        data = r.get_json()
        self.assertEqual(data["playlist"]["title"], "On Air")
        self.assertEqual(data["playlist"]["description"], "Pour la route")
        self.assertEqual(data["playlist"]["owner"], "alice")
        self.assertEqual(data["playlist"]["nb_tracks"], 3)  # bob's upload never listed
        tracks = {t["title"]: t for t in data["tracks"]}
        self.assertNotIn("Their Upload", tracks)
        # The playlist's cover is the first track's capability URL.
        self.assertEqual(
            data["playlist"]["cover"], f"/api/shared/playlist/{link['token']}/cover/1"
        )
        a = tracks["Archived One"]
        self.assertEqual(a["mid"], "1")  # the Deezer id when there is one
        self.assertTrue(a["streamable"])
        self.assertEqual(a["cover"], f"/api/shared/playlist/{link['token']}/cover/1")
        self.assertEqual(a["artist"]["name"], "Deezer Artist")
        c = tracks["Cold One"]
        self.assertEqual(c["mid"], "2")
        self.assertFalse(c["streamable"])  # listed, but not playable
        m = tracks["My Upload"]
        self.assertEqual(m["mid"], str(mine.id))  # a local row: its UUID
        self.assertTrue(m["local"])
        self.assertTrue(m["streamable"])
        # The public page costs 5 statements whatever the playlist holds: the
        # share row, the playlist, one tuples SELECT of the tracks, one batched
        # credits query, and the owner's name.
        with count_statements() as stmts:
            public.get(f"/api/shared/playlist/{link['token']}")
        self.assertEqual(sum(stmts.values()), 5, dict(stmts))

    def test_a_tampered_token_is_404(self):
        pl = self._playlist(self.alice)
        self._deezer_track("1", archived=True)
        link = self._make_link(self.owner, pl)
        rid, _ = link["token"].split(".")
        forged = rid + "." + "A" * 24
        self.assertEqual(self._public().get(f"/api/shared/playlist/{forged}").status_code, 404)
        # The row id alone opens nothing either.
        self.assertEqual(self._public().get(f"/api/shared/playlist/{rid}").status_code, 404)
        self.assertEqual(
            self._public().get("/api/shared/playlist/not/a/token").status_code, 404
        )

    def test_a_folder_cover_is_served_to_the_public_page(self):
        # The fallback art (a cover.jpg beside the file, no embedded image) was
        # the old send_file(CoverFile) bug: the route answered 404/500. The
        # public page must be able to show it like any other cover.
        t = self._local_track("Folder Art")
        folder = os.path.dirname(t.path)
        with open(os.path.join(folder, "cover.jpg"), "wb") as fh:
            fh.write(_jpeg())
        pl = self._playlist(self.alice)
        pl.add(t)
        link = self._make_link(self.owner, pl)
        r = self._public().get(
            f"/api/shared/playlist/{link['token']}/cover/{t.id}"
        )
        self.assertEqual(r.status_code, 200)
        self.assertEqual(r.mimetype, "image/jpeg")
        self.assertTrue(r.get_data().startswith(b"\xff\xd8\xff"))

    def test_the_stream_serves_the_archived_bytes_with_ranges(self):
        t = self._deezer_track("1", "Archived One", archived=True)
        pl = self._playlist(self.alice)
        pl.add(t)
        link = self._make_link(self.owner, pl)
        public = self._public()
        with open(t.path, "rb") as fh:
            blob = fh.read()
        r = public.get(f"/api/shared/playlist/{link['token']}/stream/1")
        self.assertEqual(r.status_code, 200)
        self.assertEqual(r.get_data(), blob)
        self.assertEqual(r.headers["Accept-Ranges"], "bytes")
        r = public.get(
            f"/api/shared/playlist/{link['token']}/stream/1",
            headers={"Range": "bytes=2-5"},
        )
        self.assertEqual(r.status_code, 206)
        self.assertEqual(r.get_data(), blob[2:6])
        self.assertEqual(r.headers["Content-Range"], f"bytes 2-5/{len(blob)}")

    def test_the_stream_refuses_what_is_not_archived_or_not_a_member(self):
        t = self._deezer_track("1", archived=True)
        cold = self._deezer_track("2", archived=False)
        outsider = self._deezer_track("3", archived=True)
        theirs = self._local_track("Their Upload", owner=self.bob)
        pl = self._playlist(self.alice)
        for t_ in (t, cold, theirs):
            pl.add(t_)
        link = self._make_link(self.owner, pl)
        public = self._public()
        # Not on disk: a clear refusal, never a download attempt.
        r = public.get(f"/api/shared/playlist/{link['token']}/stream/2")
        self.assertEqual(r.status_code, 409)
        self.assertEqual(r.get_json()["error"], "not archived")
        # Not in this playlist, or somebody else's private upload: 404.
        self.assertEqual(
            public.get(f"/api/shared/playlist/{link['token']}/stream/3").status_code, 404
        )
        self.assertEqual(
            public.get(f"/api/shared/playlist/{link['token']}/stream/{outsider.id}").status_code,
            404,
        )
        self.assertEqual(
            public.get(f"/api/shared/playlist/{link['token']}/stream/{theirs.id}").status_code,
            404,
        )

    def test_the_cover_works_only_for_playlist_members(self):
        t = self._deezer_track("1", "With Art", archived=True)
        self._cover(t)
        cold = self._deezer_track("2", "Cold One", archived=False)
        outsider = self._deezer_track("3", "Outsider", archived=True)
        self._cover(outsider)
        pl = self._playlist(self.alice)
        pl.add(t)
        pl.add(cold)
        link = self._make_link(self.owner, pl)
        public = self._public()
        r = public.get(f"/api/shared/playlist/{link['token']}/cover/1")
        self.assertEqual(r.status_code, 200)
        self.assertEqual(r.get_data(), _jpeg())
        # Its album's art too (an album of this playlist), from the same source.
        r = public.get(f"/api/shared/playlist/{link['token']}/cover/{t.album.id}")
        self.assertEqual(r.status_code, 200)
        self.assertEqual(r.get_data(), _jpeg())
        # A track that is not in the playlist is not a cover source.
        self.assertEqual(
            public.get(f"/api/shared/playlist/{link['token']}/cover/3").status_code, 404
        )
        # An unarchived Deezer member has no public cover: no provider call.
        self.assertEqual(
            public.get(f"/api/shared/playlist/{link['token']}/cover/2").status_code, 404
        )
        # And a dead link serves nothing.
        self.owner.delete(f"/api/playlist/{pl.id}/share")
        self.assertEqual(
            public.get(f"/api/shared/playlist/{link['token']}/cover/1").status_code, 404
        )

    def test_the_stream_is_rate_limited_per_token_and_ip(self):
        from neezer.webui.playlist_share import _stream_limiter

        t = self._deezer_track("1", archived=True)
        pl = self._playlist(self.alice)
        pl.add(t)
        link = self._make_link(self.owner, pl)
        public = self._public()
        path = f"/api/shared/playlist/{link['token']}/stream/1"
        # 120 requests pass in the window, the 121st is refused with a delay.
        with mock.patch.object(_stream_limiter, "max_requests", 3):
            for _ in range(3):
                self.assertEqual(public.get(path).status_code, 200)
            r = public.get(path)
            self.assertEqual(r.status_code, 429)
            self.assertTrue(int(r.headers["Retry-After"]) > 0)

    def test_no_public_or_owner_route_touches_the_provider(self):
        spy = mock.MagicMock()
        self.app.deezer = spy
        t = self._deezer_track("1", archived=True)
        self._cover(t)
        cold = self._deezer_track("2", archived=False)
        pl = self._playlist(self.alice)
        pl.add(t)
        pl.add(cold)
        link = self._make_link(self.owner, pl)
        self.owner.get(f"/api/playlist/{pl.id}/share")
        public = self._public()
        token = link["token"]
        public.get(f"/api/shared/playlist/{token}")
        public.get(f"/api/shared/playlist/{token}/stream/1")
        public.get(f"/api/shared/playlist/{token}/cover/1")
        public.get(f"/api/shared/playlist/{token}/cover/{t.album.id}")
        public.get(f"/api/shared/playlist/{token}/stream/2")
        self.owner.delete(f"/api/playlist/{pl.id}/share")
        self.assertEqual(spy.mock_calls, [])

    def test_the_short_link_page_previews_and_never_raises(self):
        pl = self._playlist(self.alice, name="Vitesse", comment=None)
        link = self._make_link(self.owner, pl)
        token = link["token"]
        r = self.app.test_client().get(f"/pl/{token}")
        self.assertEqual(r.status_code, 200)
        page = r.get_data(as_text=True)
        self.assertIn("Vitesse", page)
        self.assertIn("de alice", page)
        self.assertIn("no-store", r.headers["Cache-Control"])
        self.assertEqual(r.headers["X-Robots-Tag"], "noindex")
        self.assertIn(f"/app/#/shared/playlist/{token}", page)
        # A dead link shows the calm page; a malformed token is a 404.
        self.owner.delete(f"/api/playlist/{pl.id}/share")
        r = self.app.test_client().get(f"/pl/{token}")
        self.assertEqual(r.status_code, 200)
        self.assertIn("n&#x27;est plus valide", r.get_data(as_text=True))
        self.assertEqual(self.app.test_client().get("/pl/nope").status_code, 404)
        self.assertEqual(self.app.test_client().get("/pl/abc.def").status_code, 404)

    def test_deleting_the_playlist_or_the_user_kills_the_links(self):
        t = self._deezer_track("1", archived=True)
        pl = self._playlist(self.alice)
        pl.add(t)
        link = self._make_link(self.owner, pl)
        self.assertEqual(PlaylistShare.select().count(), 1)
        pl.clear()  # playlist_track has no cascade of its own
        Playlist.delete().where(Playlist.id == pl.id).execute()
        self.assertEqual(PlaylistShare.select().count(), 0)  # CASCADE
        # And the public page reads like a dead link.
        self.assertEqual(
            self._public().get(f"/api/shared/playlist/{link['token']}").status_code, 404
        )
