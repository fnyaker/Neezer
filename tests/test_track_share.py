# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""The public link to ONE track (neezer/webui/trackshare.py).

The link is a capability: a copy of the table opens nothing, the token carries
an HMAC under the app's secret, a revocation is dead on the very next public
request, and the public side never asks a provider. What these tests pin, with
numbers: the page is one small HTML response with the Open Graph/oEmbed tags a
messaging app reads, the stream is Opus (never the archived FLAC, never an MP3),
and the stream limiter answers 429 after its window.
"""

import gzip
import io
import os
import re
import shutil
import subprocess
import tempfile
import unittest
from datetime import timedelta
from unittest import mock

from neezer.config import DefaultConfig
from neezer.db import TrackShare, User, now, release_database
from neezer.managers.user import UserManager
from neezer.web import create_application

from .test_deezer import count_statements


def _jpeg():
    """A real, tiny JPEG — the only shape ``covers.is_valid_cover`` accepts."""
    from PIL import Image

    buf = io.BytesIO()
    Image.new("RGB", (12, 12), "white").save(buf, "JPEG")
    return buf.getvalue()


def _flac(path, seconds=2):
    """A real FLAC, so the transcode is ffmpeg's answer and not a stub's."""
    os.makedirs(os.path.dirname(path), exist_ok=True)
    subprocess.run(
        [
            "ffmpeg",
            "-v",
            "0",
            "-f",
            "lavfi",
            "-i",
            f"sine=frequency=440:duration={seconds}",
            "-c:a",
            "flac",
            "-y",
            path,
        ],
        check=True,
        timeout=60,
    )


@unittest.skipUnless(shutil.which("ffmpeg"), "ffmpeg is required for the Opus stream")
class TrackShareTestCase(unittest.TestCase):
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
                self.WEBAPP = dict(
                    self.WEBAPP, cache_dir=cache, mount_webui=True, mount_api=True
                )

        self.app = create_application(Config())
        UserManager.add("alice", "Alic3")
        UserManager.add("bob", "B0bbb")
        self.alice = User.get(name="alice")
        self.bob = User.get(name="bob")

        from neezer.webui.trackshare import _clear_limiters

        _clear_limiters()

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
        from neezer.deezer import library

        root = library.get_root_folder(self.archive)
        t = library.upsert_track(
            {
                "SNG_ID": sng_id,
                "SNG_TITLE": title,
                "ART_NAME": "Artiste Test",
                "ART_ID": "400",
                "ALB_TITLE": "Album Test",
                "ALB_ID": "410",
                "DURATION": "200",
                "TRACK_NUMBER": "1",
                "DISK_NUMBER": "1",
            },
            root,
        )
        if archived:
            _flac(t.path)
            t.last_modification = 1700000000
            t.has_art = True
            t.save()
            library.save_album_cover(t.folder, _jpeg())
        return t

    def _local_track(self, title, owner=None):
        from neezer.deezer import library, local

        root = library.get_root_folder(self.archive)
        folder = os.path.join(self.archive, "Local", title.replace(" ", "_"))
        os.makedirs(folder, exist_ok=True)
        path = os.path.join(folder, "song.flac")
        _flac(path)

        class FakeTag:
            artist = "Local Band"
            albumartist = None
            album = "Demo"
            genre = "Indie"
            disc = 1
            track = 1
            year = None
            length = 2.0
            bitrate = 900000
            images = []

        FakeTag.title = title
        orig = local._load_tag
        local._load_tag = lambda p: FakeTag() if p == path else None
        try:
            return local.import_local_file(path, root, owner=owner)
        finally:
            local._load_tag = orig

    def _make_link(self, client, track, body=None):
        r = client.post(f"/api/track/{track.id}/share", json=body or {})
        self.assertEqual(r.status_code, 200, r.get_json())
        return r.get_json()["link"]

    def _public(self):
        return self.app.test_client()

    # -- the owner's lifecycle ----------------------------------------------

    def test_owner_creates_reads_and_revokes_a_link(self):
        t = self._deezer_track("101")
        self.assertIsNone(self.owner.get(f"/api/track/{t.id}/share").get_json()["link"])
        link = self._make_link(self.owner, t)
        rid, mac = link["token"].split(".")
        self.assertEqual(len(mac), 24)  # a token, not the bare row id
        self.assertNotEqual(mac, "0" * 24)
        self.assertEqual(link["url"], f"/t/{link['token']}")
        self.assertIsNotNone(link["expires"])
        self.assertIsNotNone(link["created"])
        # One live link per track: asking again hands the same one back.
        again = self._make_link(self.owner, t)
        self.assertEqual(again["token"], link["token"])
        self.assertEqual(TrackShare.select().count(), 1)
        # The public page works, then the cut kills it on the very next one.
        self.assertEqual(self._public().get(link["url"]).status_code, 200)
        self.assertEqual(
            self.owner.delete(f"/api/track/{t.id}/share").get_json(), {"revoked": 1}
        )
        self.assertEqual(self._public().get(link["url"]).status_code, 404)
        self.assertIsNone(self.owner.get(f"/api/track/{t.id}/share").get_json()["link"])
        # A new link is a NEW id and mac: the old one stays dead.
        fresh = self._make_link(self.owner, t)
        self.assertNotEqual(fresh["id"], link["id"])
        self.assertEqual(self._public().get(f"/t/{link['token']}").status_code, 404)
        self.assertEqual(self._public().get(fresh["url"]).status_code, 200)

    def test_a_forged_expired_or_disabled_link_is_the_same_404(self):
        t = self._deezer_track("102")
        link = self._make_link(self.owner, t)
        rid, mac = link["token"].split(".")
        forged = rid + "." + ("A" * 24)
        if forged == link["token"]:
            forged = rid + "." + ("B" * 24)
        for path in (
            f"/t/{forged}",
            f"/api/shared/track/{forged}/cover",
            f"/api/shared/track/{forged}/stream",
        ):
            r = self._public().get(path)
            self.assertEqual(r.status_code, 404, path)
            self.assertEqual(r.headers.get("X-Robots-Tag"), "noindex, nofollow")
            self.assertEqual(r.headers.get("Cache-Control"), "no-store")

        # Expiry and the owner's disabled flag both read as the same dead link.
        TrackShare.update(expires=now() - timedelta(seconds=1)).where(
            TrackShare.id == link["id"]
        ).execute()
        self.assertEqual(self._public().get(link["url"]).status_code, 404)
        TrackShare.update(expires=None).where(TrackShare.id == link["id"]).execute()
        User.update(disabled=now()).where(User.id == self.alice.id).execute()
        self.assertEqual(self._public().get(link["url"]).status_code, 404)

    def test_one_track_only_and_no_enumeration(self):
        a = self._deezer_track("103", title="Partagée")
        b = self._deezer_track("104", title="Jamais partagée")
        link = self._make_link(self.owner, a)
        page = self._public().get(link["url"]).get_data(as_text=True)
        self.assertIn("Partagée", page)
        self.assertNotIn("Jamais partagée", page)
        # No route accepts a media id from the visitor: a sibling is unreachable.
        self.assertEqual(
            self._public()
            .get(f"/api/shared/track/{link['token']}/stream/{b.id}")
            .status_code,
            404,
        )
        self.assertEqual(
            self._public()
            .get(f"/api/shared/track/{link['token']}/cover/{b.id}")
            .status_code,
            404,
        )

    def test_a_private_upload_of_somebody_else_is_neither_shared_nor_served(self):
        theirs = self._local_track("Privé de Bob", owner=self.bob)
        r = self.owner.post(f"/api/track/{theirs.id}/share", json={})
        self.assertEqual(r.status_code, 404)
        self.assertEqual(TrackShare.select().count(), 0)
        # Alice's own upload is hers to publish, and the public link serves it.
        mine = self._local_track("Mon upload", owner=self.alice)
        link = self._make_link(self.owner, mine)
        self.assertEqual(self._public().get(link["url"]).status_code, 200)

    def test_the_list_only_shows_the_callers_live_links(self):
        a = self._deezer_track("105", title="À Alice")
        b = self._deezer_track("106", title="À Bob")
        alice_link = self._make_link(self.owner, a)
        bob_link = self._make_link(self.other, b)

        mine = self.owner.get("/api/track-shares")
        self.assertEqual(mine.status_code, 200)
        links = mine.get_json()["links"]
        self.assertEqual([row["id"] for row in links], [alice_link["id"]])
        self.assertEqual(links[0]["track"]["title"], "À Alice")
        self.assertEqual(links[0]["track"]["artist"], "Artiste Test")
        self.assertNotIn(bob_link["id"], [row["id"] for row in links])

        # Somebody else's link reads as not found, and stays alive.
        self.assertEqual(
            self.owner.delete(f"/api/track-shares/{bob_link['id']}").status_code, 404
        )
        self.assertEqual(self._public().get(bob_link["url"]).status_code, 200)
        self.assertEqual(
            self.other.delete(f"/api/track-shares/{bob_link['id']}").status_code, 200
        )
        self.assertEqual(self._public().get(bob_link["url"]).status_code, 404)

    def test_an_unarchived_track_never_creates_a_link(self):
        t = self._deezer_track("107", archived=False)
        r = self.owner.post(f"/api/track/{t.id}/share", json={})
        self.assertIn(r.status_code, (409, 502, 503), r.get_json())
        self.assertEqual(TrackShare.select().count(), 0)

    # -- the public page -----------------------------------------------------

    def test_the_page_carries_the_preview_and_stays_cheap(self):
        t = self._deezer_track("108", title="Un titre avec un nom très long pour l'aperçu")
        link = self._make_link(self.owner, t)
        public = self._public()
        with count_statements() as stmts:
            r = public.get(link["url"])
        page = r.get_data(as_text=True)
        self.assertEqual(r.status_code, 200)
        self.assertEqual(r.headers.get("Content-Type"), "text/html; charset=utf-8")
        self.assertEqual(r.headers.get("Cache-Control"), "no-store")
        self.assertEqual(r.headers.get("X-Robots-Tag"), "noindex, nofollow")
        self.assertEqual(r.headers.get("Referrer-Policy"), "no-referrer")
        self.assertIn("og:type' content='music.song", page)
        self.assertIn("og:title", page)
        self.assertIn("og:image", page)
        self.assertIn("og:audio", page)
        self.assertIn("audio/ogg", page)
        self.assertIn("music:duration", page)
        self.assertIn("twitter:player", page)
        self.assertIn("application/json+oembed", page)
        self.assertIn("Titre partagé", page)
        self.assertNotIn(self.archive, page)
        self.assertNotIn("ffmpeg", page.lower())
        self.assertNotIn("deezer.com", page.lower())
        # Enough detail for a messaging app to build a card, not a whole app.
        raw = len(r.data)
        packed = len(gzip.compress(r.data))
        self.assertLessEqual(raw, 13_000, f"{raw} B raw")
        self.assertLessEqual(packed, 4_200, f"{packed} B gzip")
        self.assertLessEqual(sum(stmts.values()), 6, stmts)
        # The inline script is allowed by ITS response's nonce, not by a blanket
        # 'unsafe-inline'.
        nonce = re.search(r"script-src 'nonce-([^']+)'", r.headers["Content-Security-Policy"])
        self.assertIsNotNone(nonce)
        self.assertIn(f"<script nonce='{nonce.group(1)}'>", page)
        self.assertNotIn("script-src 'unsafe-inline'", r.headers["Content-Security-Policy"])

    def test_a_dead_link_is_the_same_calm_page_whatever_the_cause(self):
        t = self._deezer_track("109")
        link = self._make_link(self.owner, t)
        public = self._public()
        live = public.get(link["url"]).get_data(as_text=True)
        self.owner.delete(f"/api/track/{t.id}/share")
        revoked = public.get(link["url"])
        malformed = public.get("/t/not-a-token")
        mac = link["token"].split(".")[1]
        forged_mac = "B" * 24 if mac != "B" * 24 else "C" * 24
        forged = public.get("/t/" + link["token"].split(".")[0] + "." + forged_mac)
        for r in (revoked, malformed, forged):
            self.assertEqual(r.status_code, 404)
            self.assertNotEqual(r.get_data(as_text=True), live)
            self.assertIn("Ce lien n’est plus valide.", r.get_data(as_text=True))
        self.assertEqual(revoked.get_data(), malformed.get_data())
        self.assertEqual(revoked.get_data(), forged.get_data())

    def test_the_cover_is_a_resized_jpeg_of_the_archived_art(self):
        t = self._deezer_track("110")
        link = self._make_link(self.owner, t)
        r = self._public().get(f"/api/shared/track/{link['token']}/cover?w=400")
        self.assertEqual(r.status_code, 200)
        self.assertEqual(r.headers.get("Content-Type"), "image/jpeg")
        from PIL import Image

        self.assertLessEqual(Image.open(io.BytesIO(r.data)).width, 400)
        self.assertGreater(len(r.data), 0)
        r.close()

    def test_oembed_describes_the_embed_and_refuses_other_urls(self):
        t = self._deezer_track("111", title="Titre oEmbed")
        link = self._make_link(self.owner, t)
        page = self._public()
        url = "http://localhost" + link["url"]
        r = page.get("/api/oembed", query_string={"url": url})
        self.assertEqual(r.status_code, 200)
        body = r.get_json()
        self.assertEqual(body["version"], "1.0")
        self.assertEqual(body["type"], "rich")
        self.assertEqual(body["provider_name"], "Neezer")
        self.assertIn("Titre oEmbed", body["title"])
        self.assertIn("<iframe", body["html"])
        self.assertIn("embed=1", body["html"])
        self.assertEqual(body["width"], 480)
        self.assertEqual(body["height"], 180)
        self.assertIn("thumbnail_url", body)
        # The endpoint parses OUR page shape; it never fetches the URL.
        self.assertEqual(
            page.get("/api/oembed", query_string={"url": "https://example.test/x"}).status_code,
            404,
        )
        self.assertEqual(
            page.get(
                "/api/oembed",
                query_string={"url": "https://example.test/t/" + link["token"] + "?x=1"},
            ).status_code,
            404,
        )
        self.assertEqual(
            page.get(
                "/api/oembed",
                query_string={"url": url, "format": "xml"},
            ).status_code,
            501,
        )
        self.owner.delete(f"/api/track/{t.id}/share")
        self.assertEqual(page.get("/api/oembed", query_string={"url": url}).status_code, 404)

    # -- the stream ----------------------------------------------------------

    def test_the_stream_is_opus_and_never_the_flac_or_an_mp3(self):
        t = self._deezer_track("112")
        link = self._make_link(self.owner, t)
        # No provider call is even reachable from the public side.
        with mock.patch(
            "neezer.deezer.archive.ensure_archived",
            side_effect=AssertionError("the public stream asked a provider"),
        ):
            r = self._public().get(
                f"/api/shared/track/{link['token']}/stream?q=OPUS_128"
            )
            self.assertEqual(r.status_code, 200)
            body = r.data
        self.assertEqual(r.headers.get("Content-Type"), "audio/ogg")
        self.assertTrue(body.startswith(b"OggS"), body[:8])
        self.assertFalse(body.startswith(b"fLaC"))
        self.assertFalse(body.startswith(b"ID3"))
        self.assertNotIn(b"audio/mpeg", r.headers.get("Content-Type", "").encode())
        # The second answer is the cached FILE, so a player can seek it.
        again = self._public().get(
            f"/api/shared/track/{link['token']}/stream?q=OPUS_128"
        )
        self.assertEqual(again.status_code, 200)
        self.assertEqual(again.headers.get("Accept-Ranges"), "bytes")
        self.assertEqual(again.data[:4], b"OggS")
        self.assertEqual(again.headers.get("Cache-Control"), "no-store")
        rng = self._public().get(
            f"/api/shared/track/{link['token']}/stream?q=OPUS_128",
            headers={"Range": "bytes=0-99"},
        )
        self.assertEqual(rng.status_code, 206)
        self.assertTrue(rng.headers.get("Content-Range", "").startswith("bytes 0-99/"))
        self.assertEqual(len(rng.data), 100)
        r.close()
        again.close()
        rng.close()

    def test_the_stream_limiter_stops_a_burst(self):
        from neezer.webui import trackshare

        t = self._deezer_track("113")
        link = self._make_link(self.owner, t)
        public = self._public()
        path = f"/api/shared/track/{link['token']}/stream"
        old = trackshare._stream_limiter.max_requests
        trackshare._stream_limiter.max_requests = 2
        try:
            self.assertEqual(public.get(path).status_code, 200)
            self.assertEqual(public.get(path).status_code, 200)
            third = public.get(path)
            self.assertEqual(third.status_code, 429)
            self.assertEqual(third.headers.get("Cache-Control"), "no-store")
            self.assertGreaterEqual(int(third.headers["Retry-After"]), 1)
        finally:
            trackshare._stream_limiter.max_requests = old

    def test_a_custom_ttl_is_bounded_and_null_means_forever(self):
        t = self._deezer_track("114")
        # The sheet's default is a month.
        link = self._make_link(self.owner, t)
        self.assertIsNotNone(link["expires"])
        # Explicit null is the owner asking for a permanent link.
        self.owner.delete(f"/api/track/{t.id}/share")
        forever = self._make_link(self.owner, t, {"ttl": None})
        self.assertIsNone(forever["expires"])
        # Junk and out-of-range values are refused, and create nothing.
        self.owner.delete(f"/api/track/{t.id}/share")
        for body in ({"ttl": 10}, {"ttl": "soon"}, {"ttl": 10**12}):
            r = self.owner.post(f"/api/track/{t.id}/share", json=body)
            self.assertEqual(r.status_code, 400, body)
        self.assertEqual(
            TrackShare.select().where(TrackShare.revoked.is_null(True)).count(), 0
        )


class TrackShareSchemaTestCase(unittest.TestCase):
    """The track_share table's two cascades, on SQLite (the external module
    proves the same shape on a real PostgreSQL and MySQL)."""

    def setUp(self):
        self.__db = tempfile.mkstemp()
        self.__dir = tempfile.mkdtemp()
        db_path, cache_dir = self.__db[1], self.__dir

        class Config(DefaultConfig):
            TESTING = True

            def __init__(self):
                super().__init__()
                self.BASE = dict(self.BASE, database_uri="sqlite:///" + db_path)
                self.WEBAPP = dict(self.WEBAPP, cache_dir=cache_dir)

        self.app = create_application(Config())
        UserManager.add("alice", "Alic3", admin=True)
        self.alice = User.get(name="alice")

    def tearDown(self):
        release_database()
        shutil.rmtree(self.__dir, ignore_errors=True)
        os.close(self.__db[0])
        os.remove(self.__db[1])

    def test_a_deleted_track_or_account_takes_its_links_with_it(self):
        from neezer.deezer import library

        root = library.get_root_folder(self.__dir)
        t = library.upsert_track(
            {
                "SNG_ID": "200",
                "SNG_TITLE": "T",
                "ART_NAME": "A",
                "ART_ID": "401",
                "ALB_TITLE": "B",
                "ALB_ID": "411",
                "DURATION": "10",
                "TRACK_NUMBER": "1",
                "DISK_NUMBER": "1",
            },
            root,
        )
        TrackShare.create(id="a" * 16, track=t, user=self.alice)
        TrackShare.create(id="b" * 16, track=t, user=self.alice)
        self.assertEqual(TrackShare.select().count(), 2)
        t.delete_instance()
        self.assertEqual(TrackShare.select().count(), 0)

        t2 = library.upsert_track(
            {
                "SNG_ID": "201",
                "SNG_TITLE": "T2",
                "ART_NAME": "A",
                "ART_ID": "401",
                "ALB_TITLE": "B",
                "ALB_ID": "411",
                "DURATION": "10",
                "TRACK_NUMBER": "2",
                "DISK_NUMBER": "1",
            },
            root,
        )
        TrackShare.create(id="c" * 16, track=t2, user=self.alice)
        self.alice.delete_instance()
        self.assertEqual(TrackShare.select().count(), 0)
