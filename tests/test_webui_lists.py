# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""The big lists' fast path, and the validator every JSON answer carries.

A playlist or the favourites is read as TUPLES (``webui._db_track_rows``)
instead of three model objects per row — 4x faster on 4000 tracks, measured
with tools/perf_api.py. That is only worth anything if the answer is the SAME,
byte for byte, as the model path's (``_db_track``): every shape a row can
take is here — a Deezer track with credits, one with none, a local upload, an
unavailable one, one with no gain, one whose artist has no Deezer id — and the
two are compared field by field. The other two rewrites (my playlists in four
statements instead of two per playlist, the favourite ids without hydrating a
row) are held to the same answers and to their statement counts.
"""

import collections
import contextlib
import os
from datetime import datetime, timedelta
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
    PlaylistMember,
    PlaylistTrack,
    PodcastChannel,
    PodcastEpisode,
    PodcastTranscript,
    StarredTrack,
    Track,
    User,
    db,
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


def raw(sid, title, art, alb, artists=None, gain=None):
    t = {
        "SNG_ID": str(sid),
        "SNG_TITLE": title,
        "ART_ID": art[0],
        "ART_NAME": art[1],
        "ALB_ID": alb[0],
        "ALB_TITLE": alb[1],
        "ALB_PICTURE": alb[2],
        "DURATION": 200 + sid % 7,
        "TRACK_NUMBER": 1,
        "DISK_NUMBER": 1,
        "TRACK_TOKEN": f"tok{sid}",
    }
    if gain is not None:
        t["GAIN"] = gain
    if artists:
        t["ARTISTS"] = artists
    return t


class ListsTestCase(unittest.TestCase):
    def setUp(self):
        self.__db = tempfile.mkstemp()
        self.__dir = tempfile.mkdtemp()
        self.archive = tempfile.mkdtemp()
        db_path = self.__db[1]
        cache = self.__dir
        archive = self.archive

        class Config(DefaultConfig):
            TESTING = True

            def __init__(self):
                super().__init__()
                self.BASE = dict(self.BASE, database_uri="sqlite:///" + db_path)
                self.WEBAPP = dict(self.WEBAPP, cache_dir=cache, mount_webui=True, mount_api=True)
                self.DEEZER = dict(getattr(self, "DEEZER", {}), archive_dir=archive, enabled=False)

        self.app = create_application(Config())
        # Deezer switched off: every list below is the database's own answer.
        self.app.deezer = None
        UserManager.add("alice", "Alic3", admin=True)
        UserManager.add("bob", "B0bb", admin=False)
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
            A = ("11", "Édith")
            B = ("12", "Bob & Co")
            credits = [
                {"ART_ID": "11", "ART_NAME": "Édith", "ROLE_ID": "0"},
                {"ART_ID": "12", "ART_NAME": "Bob & Co", "ROLE_ID": "5"},
            ]
            t1 = library.upsert_track(raw(1, "Été", A, ("21", "Album Un", "aa11"), credits, gain="-7.5"), root)
            t2 = library.upsert_track(raw(2, "Nuit", B, ("22", "Album Deux", None)), root)
            t3 = library.upsert_track(raw(3, "Zéro", A, ("21", "Album Un", "aa11")), root)
            t3.unavailable = library._now()
            t3.save()
            # A local upload: no Deezer id anywhere, its own artist and album.
            folder = Folder.create(root=False, name="up", path=os.path.join(self.archive, "up"), parent=root)
            art = Artist.create(name="Moi")
            alb = Album.create(name="Maison", artist=art)
            t4 = Track.create(
                disc=1, number=1, title="Démo", duration=61, album=alb, artist=art, bitrate=320,
                path=os.path.join(self.archive, "up", "demo.mp3"), last_modification=0,
                root_folder=root, folder=folder,
            )
            # An artist the importer made without a Deezer id.
            nobody = Artist.create(name="Sans id")
            t5 = library.upsert_track(raw(5, "Orphelin", ("13", "X"), ("23", "Alb", "bb22")), root)
            t5.artist = nobody
            t5.save()
            self.tracks = [t1, t2, t3, t4, t5]
            alice = User.get(name="alice")
            bob = User.get(name="bob")
            # Timestamps are whole seconds (db.now): spell out an order, or
            # "newest first" is a tie the database breaks as it likes.
            at = lambda k: datetime(2026, 1, 1) + timedelta(minutes=k)  # noqa: E731
            for k, t in enumerate(self.tracks):
                StarredTrack.create(user=alice, starred=t.id, date=at(k))
                if k % 2 == 0:
                    StarredTrack.create(user=bob, starred=t.id, date=at(k))
            self.big = Playlist.create(user=alice, name="Big", created=at(1))
            for k, t in enumerate(self.tracks + self.tracks[::-1]):
                PlaylistTrack.create(playlist=self.big, track=t.id, index=k)
            self.local_first = Playlist.create(user=alice, name="Local first", created=at(2))
            PlaylistTrack.create(playlist=self.local_first, track=t4.id, index=0)
            PlaylistTrack.create(playlist=self.local_first, track=t1.id, index=1)
            self.empty = Playlist.create(user=alice, name="Empty", created=at(3))

    def _login(self, who="alice", pw="Alic3"):
        rv = self.client.post("/api/login", json={"username": who, "password": pw})
        self.assertEqual(rv.status_code, 200)

    def test_the_tuple_path_answers_what_the_model_path_did(self):
        from neezer import webui

        with self.app.test_request_context():
            q = webui._db_playlist_track_rows(self.big)
            fast = webui._db_tracks(q)
            slow = webui._db_tracks(list(q))  # a list: the model path
        self.assertEqual(len(fast), 10)
        self.assertEqual(fast, slow)
        by_id = {t["deezer_id"]: t for t in fast}
        # The shapes this covers, so a fixture change cannot quietly drop one.
        self.assertEqual([a["name"] for a in by_id["1"]["artists"]], ["Édith", "Bob & Co"])
        self.assertEqual(by_id["1"]["display_artist"], "Édith feat. Bob & Co")
        self.assertEqual(by_id["1"]["gain"], -7.5)
        # Every row's cover is its album's local route, md5 or not.
        self.assertEqual(by_id["2"]["album"]["cover"], "/api/cover/" + str(self.tracks[1].album_id))
        self.assertEqual(by_id["2"]["id"], str(self.tracks[1].id))
        self.assertIn("release_date", by_id["2"])
        self.assertTrue(by_id["3"]["unavailable"])
        local = by_id[str(self.tracks[3].id)]
        self.assertTrue(local["local"])
        self.assertEqual(local["id"], str(self.tracks[3].id))
        self.assertEqual(local["album"]["cover"], "/api/cover/" + str(self.tracks[3].album_id))
        self.assertEqual(local["artist"]["deezer_id"], str(self.tracks[3].artist.id))
        self.assertEqual(by_id["5"]["artists"][0]["deezer_id"], str(self.tracks[4].artist.id))

    def test_a_query_with_a_limit_reads_its_credits_by_id(self):
        # MySQL refuses LIMIT inside IN (...): a limited query takes the id list.
        from neezer import webui

        with self.app.test_request_context():
            q = webui._db_playlist_track_rows(self.big).limit(3)
            self.assertEqual(webui._db_tracks(q), webui._db_tracks(list(q)))

    def test_favourites_are_the_same_list_in_three_statements(self):
        self._login()
        with count_statements() as counts:
            favs = self.client.get("/api/me/favorites").get_json()["tracks"]
        self.assertEqual(len(favs), 5)
        # Newest star first, as before.
        self.assertEqual(favs[0]["deezer_id"], "5")
        # The session, the list, its credits.
        self.assertLessEqual(counts["SELECT"], 3, counts)

    def test_favourite_ids_are_the_same_set(self):
        self._login()
        ids = set(self.client.get("/api/me/favorite-ids").get_json()["ids"])
        self.assertEqual(ids, {"1", "2", "3", "5", str(self.tracks[3].id)})
        self.client.post("/api/logout")
        self._login("bob", "B0bb")
        ids = self.client.get("/api/me/favorite-ids").get_json()["ids"]
        # A guest's list keeps its order: newest first.
        self.assertEqual(ids, ["5", "3", "1"])

    def test_my_playlists_counts_and_covers_in_six_statements(self):
        self._login()
        with count_statements() as counts:
            answer = self.client.get("/api/me/playlists").get_json()
        pls = answer["playlists"]
        by_name = {p["title"]: p for p in pls}
        self.assertEqual(by_name["Big"]["nb_tracks"], 10)
        self.assertEqual(by_name["Big"]["cover"], "/api/cover/" + str(self.tracks[0].album_id))
        # A local first track: its album's cover route, like any other.
        self.assertEqual(by_name["Local first"]["cover"], "/api/cover/" + str(self.tracks[3].album_id))
        # A playlist that was never ordered: unfiled, and no position at all.
        # `role`/`can_manage`/`owner` are the collaborative lot's additive keys.
        self.assertEqual(by_name["Empty"], {
            "id": str(self.empty.id), "deezer_id": None, "title": "Empty", "cover": None,
            "nb_tracks": 0, "editable": True, "folder_id": None, "position": None,
            "role": "owner", "can_manage": True, "owner": "alice",
        })
        self.assertEqual(answer["folders"], [])
        # Newest first, as the model path listed them.
        self.assertEqual([p["title"] for p in pls], ["Empty", "Local first", "Big"])
        # The session, the playlists, their counts, their first tracks, the
        # folder list and the folder counts — not two per playlist.
        self.assertLessEqual(counts["SELECT"], 6, counts)

    def test_an_unchanged_answer_is_revalidated_not_resent(self):
        self._login()
        rv = self.client.get("/api/me/favorites")
        self.assertEqual(rv.status_code, 200)
        tag = rv.headers["ETag"]
        self.assertTrue(tag.startswith('W/"'), tag)
        self.assertEqual(rv.headers["Cache-Control"], "private, no-cache")
        again = self.client.get("/api/me/favorites", headers={"If-None-Match": tag})
        self.assertEqual(again.status_code, 304)
        self.assertEqual(again.get_data(), b"")
        # Something changed: a full answer, with another tag.
        with self.app.app_context():
            StarredTrack.delete().where(StarredTrack.starred == self.tracks[0].id).execute()
        changed = self.client.get("/api/me/favorites", headers={"If-None-Match": tag})
        self.assertEqual(changed.status_code, 200)
        self.assertNotEqual(changed.headers["ETag"], tag)

    def test_the_validator_survives_compression(self):
        self._login()
        rv = self.client.get("/api/playlist/" + str(self.big.id), headers={"Accept-Encoding": "gzip"})
        self.assertEqual(rv.headers.get("Content-Encoding"), "gzip")
        tag = rv.headers["ETag"]
        rv = self.client.get(
            "/api/playlist/" + str(self.big.id), headers={"Accept-Encoding": "gzip", "If-None-Match": tag}
        )
        self.assertEqual(rv.status_code, 304)
        self.assertIsNone(rv.headers.get("Content-Encoding"))

    def test_only_json_gets_carry_a_validator(self):
        self._login()
        rv = self.client.post("/api/listen", json={"id": "1", "seconds": 5})
        self.assertIsNone(rv.headers.get("ETag"))
        rv = self.client.get("/api/me/playlists", headers={"If-None-Match": '"nope"'})
        self.assertEqual(rv.status_code, 200)

    def test_an_artists_album_grid_is_counted_once_not_per_album(self):
        from neezer.deezer import library

        with self.app.app_context():
            root = library.get_root_folder(self.archive)
            A = ("11", "Édith")
            for k in range(6):
                for n in range(k + 1):
                    t = library.upsert_track(raw(100 + 10 * k + n, f"T{k}{n}", A, (f"3{k}", f"Disque {k}", None)), root)
                    t.year = 1990 + k + n
                    t.save()
        self._login()
        with count_statements() as counts:
            art = self.client.get("/api/artist/11").get_json()
        cards = {c["title"]: c for c in art["albums"]}
        self.assertEqual(cards["Disque 3"]["nb_tracks"], 4)
        self.assertEqual(cards["Disque 3"]["year"], 1993)
        # The fixture's own album, whose tracks carry no year.
        self.assertEqual(cards["Album Un"]["nb_tracks"], 2)
        self.assertIsNone(cards["Album Un"]["year"])
        # Seven albums, and the grid costs the same as it would for one: the
        # count per album is gone (it was 2 statements each). Two more than it
        # was: the similar artists, read from the stored neighbours.
        self.assertLessEqual(counts["SELECT"], 10, counts)
        with count_statements() as counts:
            disco = self.client.get("/api/artist/11/discography").get_json()["discography"]
        self.assertEqual(len(disco["album"]), 7)
        self.assertEqual(disco["album"][0]["year"], 1995)  # newest first (an album is its first year)
        self.assertLessEqual(counts["SELECT"], 5, counts)

    def test_podcasts_are_counted_once_not_per_show(self):
        from neezer.db import PodcastChannel, PodcastEpisode

        with self.app.app_context():
            alice = User.get(name="alice")
            for k in range(5):
                c = PodcastChannel.create(user=alice, url=f"https://example.org/{k}", title=f"Show {k}")
                for n in range(k):
                    PodcastEpisode.create(channel=c, title=f"E{n}", path=f"/x/{k}/{n}.mp3" if n % 2 else None)
        self._login()
        with count_statements() as counts:
            shows = self.client.get("/api/podcasts").get_json()["podcasts"]
        by = {c["title"]: c for c in shows}
        self.assertEqual(by["Show 4"]["episode_count"], 4)
        self.assertEqual(by["Show 4"]["archived_count"], 2)
        self.assertEqual(by["Show 0"]["episode_count"], 0)
        self.assertEqual(by["Show 0"]["archived_count"], 0)
        self.assertLessEqual(counts["SELECT"], 3, counts)

    def _bulk_tracks(self, count):
        """``count`` rows of one album, one transaction."""
        from neezer.deezer import library

        with self.app.app_context():
            root = library.get_root_folder(self.archive)
            artist = Artist.get(Artist.name == "Sans id")
            album = Album.get(Album.name == "Maison")
            made = []
            with db.atomic():
                for i in range(count):
                    made.append(
                        Track.create(
                            disc=1,
                            number=i + 1,
                            title="Gros %04d" % i,
                            duration=60,
                            album=album,
                            artist=artist,
                            bitrate=320,
                            path=os.path.join(self.archive, "bulk", "%d.mp3" % i),
                            last_modification=0,
                            root_folder=root,
                            folder=root,
                        )
                    )
        return [str(track.id) for track in made]

    def test_creating_a_playlist_of_2000_tracks_costs_14_statements(self):
        """The route used to call ``Playlist.add`` per title — a ``MAX(index)``
        read plus an INSERT each, 4 005 statements for 2 000 tracks under the
        request thread. One ``MAX`` and seven 300-row INSERTs make it 14, and the
        stored order is still 0..n-1."""
        ids = self._bulk_tracks(2000)
        self._login()
        with count_statements() as counts:
            rv = self.client.post(
                "/api/playlists", json={"title": "Grosse", "tracks": ids}
            )
        self.assertEqual(rv.status_code, 200, rv.get_data()[:200])
        pid = rv.get_json()["id"]
        self.assertLessEqual(sum(counts.values()), 15, counts)
        with self.app.app_context():
            stored = [
                row.index
                for row in PlaylistTrack.select(PlaylistTrack.index)
                .where(PlaylistTrack.playlist == pid)
                .order_by(PlaylistTrack.index)
            ]
        self.assertEqual(stored, list(range(2000)))

        with count_statements() as counts:
            rv = self.client.post(
                "/api/playlist/%s/tracks" % pid, json={"tracks": ids}
            )
        self.assertEqual(rv.status_code, 200, rv.get_data()[:200])
        self.assertLessEqual(sum(counts.values()), 15, counts)
        self.assertEqual(rv.get_json()["added"], 2000)
        with self.app.app_context():
            stored = [
                row.index
                for row in PlaylistTrack.select(PlaylistTrack.index)
                .where(PlaylistTrack.playlist == pid)
                .order_by(PlaylistTrack.index)
            ]
        self.assertEqual(stored, list(range(4000)))

    def test_a_show_of_a_thousand_episodes_is_one_page_of_fifty(self):
        """A three-year daily show serialized every episode on every open: the
        payload grew with the catalogue (≈450 B each) and the page cost grew
        with it. One page is 50 by default, 200 at most, and the counters stay
        the show's own."""
        with self.app.app_context():
            alice = User.get(name="alice")
            show = PodcastChannel.create(
                user=alice, url="https://example.org/big", title="Grand"
            )
            with db.atomic():
                for i in range(1000):
                    episode = PodcastEpisode.create(
                        channel=show,
                        title="Épisode %04d" % i,
                        description="Une description courte.",
                        duration=600,
                        publish_date=datetime(2020, 1, 1) + timedelta(hours=i),
                        status="completed",
                    )
                    if i < 3:
                        PodcastTranscript.create(
                            episode=episode, status="ready", n_segments=1
                        )
            cid = str(show.id)
        self._login()
        with count_statements() as counts:
            rv = self.client.get("/api/podcast/" + cid)
        self.assertEqual(rv.status_code, 200)
        body = rv.get_json()
        self.assertEqual(body["episode_count"], 1000)
        self.assertEqual(body["archived_count"], 0)
        self.assertEqual(len(body["episodes"]), 50)
        self.assertEqual(body["episodes"][0]["title"], "Épisode 0999")
        self.assertEqual(body["episodes"][-1]["title"], "Épisode 0950")
        # Three transcripts exist on the three OLDEST episodes: the counter is
        # the show's, not the page's.
        self.assertEqual(body["transcribed_count"], 3)
        self.assertIsNotNone(body["next"])
        self.assertLessEqual(sum(counts.values()), 10, counts)
        self.assertLess(len(rv.get_data()), 40000)
        first_page = [e["id"] for e in body["episodes"]]

        rv = self.client.get("/api/podcast/%s?cursor=%s" % (cid, body["next"]))
        second = rv.get_json()
        self.assertEqual(len(second["episodes"]), 50)
        self.assertEqual(second["episodes"][0]["title"], "Épisode 0949")
        self.assertFalse(set(first_page) & {e["id"] for e in second["episodes"]})
        # A page boundary of a thousand episodes: 20 requests, all distinct.
        seen = list(first_page) + [e["id"] for e in second["episodes"]]
        cursor = second["next"]
        while cursor:
            page = self.client.get(
                "/api/podcast/%s?cursor=%s" % (cid, cursor)
            ).get_json()
            seen.extend(e["id"] for e in page["episodes"])
            cursor = page["next"]
        self.assertEqual(len(seen), 1000)
        self.assertEqual(len(set(seen)), 1000)

    def test_the_episode_page_takes_a_bounded_limit_and_refuses_junk(self):
        with self.app.app_context():
            alice = User.get(name="alice")
            show = PodcastChannel.create(
                user=alice, url="https://example.org/small", title="Petit"
            )
            for i in range(3):
                PodcastEpisode.create(
                    channel=show,
                    title="Ép %d" % i,
                    publish_date=datetime(2026, 1, 1) + timedelta(days=i),
                    status="completed",
                )
            cid = str(show.id)
        self._login()
        # A show that fits its page answers exactly what it used to: every
        # episode, and no next page.
        body = self.client.get("/api/podcast/" + cid).get_json()
        self.assertEqual(
            [e["title"] for e in body["episodes"]], ["Ép 2", "Ép 1", "Ép 0"]
        )
        self.assertIsNone(body["next"])
        # 200 is the cap; 1 is the smallest page.
        for query, expected in (("limit=1", 1), ("limit=200", 3)):
            page = self.client.get(
                "/api/podcast/%s?%s" % (cid, query)
            ).get_json()
            self.assertEqual(len(page["episodes"]), expected, query)
        for query in ("limit=0", "limit=201", "limit=abc", "cursor=junk",
                      "cursor=1|2",
                      "cursor=2026-01-01T00:00:00|2026-01-01T00:00:00|pas-un-uuid"):
            rv = self.client.get("/api/podcast/%s?%s" % (cid, query))
            self.assertEqual(rv.status_code, 400, query)
            self.assertIn("error", rv.get_json())


if __name__ == "__main__":
    unittest.main()
