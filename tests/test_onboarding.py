# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""The onboarding artist feed: diversity, pool/cache order, routes and writes."""

import collections
import contextlib
import hashlib
import itertools
import os
import shutil
import tempfile
import threading
import unittest
import uuid
from unittest import mock

import peewee

from neezer.config import DefaultConfig
from neezer.db import (
    Album,
    Artist,
    ArtistLink,
    ArtistMute,
    EntityRef,
    Folder,
    StarredArtist,
    Track,
    User,
    now,
    release_database,
)
from neezer.deezer import ids, onboarding_pool as pool
from neezer.deezer import youtube as yt
from deezerpy.errors import DeezerUnavailable
from neezer.managers.user import UserManager
from neezer.onboarding import (
    COUNTRIES,
    DIVERSITY,
    Card,
    diversify,
    next_after,
    pick_similar,
    rank_search,
    read_after,
    seed_for,
)
from neezer.web import create_application
from neezer.webui.onboarding import onboarding_api


@contextlib.contextmanager
def count_statements():
    counts = collections.Counter()
    original = peewee.Database.execute_sql
    mine = threading.get_ident()

    def execute_sql(self, sql, *args, **kwargs):
        if threading.get_ident() == mine:
            counts[sql.split(None, 1)[0].upper()] += 1
        return original(self, sql, *args, **kwargs)

    peewee.Database.execute_sql = execute_sql
    try:
        yield counts
    finally:
        peewee.Database.execute_sql = original


def deezer_card(ref, name=None, genres=(), countries=(), sources=("chart",), rank=1):
    return Card(
        provider="deezer",
        ref=str(ref),
        name=name or f"Artist {ref}",
        picture=f"https://img/{ref}.jpg",
        picture_small=f"https://img/{ref}-small.jpg",
        genres=tuple(genres),
        countries=tuple(countries),
        rank=rank,
        source=sources[0] if sources else "chart",
        sources=tuple(sources),
    )


def youtube_card(ref, name, countries=(), rank=1):
    return Card(
        provider="youtube",
        ref=ref,
        name=name,
        picture=f"https://yt3.googleusercontent.com/{ref}=w60-h60-p-l90-rj",
        picture_small=f"https://yt3.googleusercontent.com/{ref}=w60-h60-p-l90-rj",
        countries=tuple(countries),
        rank=rank,
        source="youtube-chart",
        sources=("youtube-chart",),
        subscribers=2_000_000,
    )


class FakeApi:
    def __init__(self):
        self.calls = []
        self.fail = False

    def _record(self, name, *args):
        self.calls.append((name, args))

    def get_genres(self, index=0, limit=10):
        self._record("get_genres", index, limit)
        if self.fail:
            raise DeezerUnavailable("down")
        return {
            "data": [
                {"id": 0, "name": "All"},
                {"id": 132, "name": "Pop"},
                {"id": 116, "name": "Rap/Hip Hop"},
                {"id": 152, "name": "Rock"},
                {"id": 113, "name": "Electro"},
                {"id": 165, "name": "R&B"},
                {"id": 129, "name": "Jazz"},
                {"id": 464, "name": "Classique"},
                {"id": 144, "name": "Reggae"},
                {"id": 106, "name": "Metal"},
                {"id": 466, "name": "Folk"},
                {"id": 98, "name": "Films/Jeux vidéo"},
            ]
        }

    def get_chart_artists(self, genre_id=0, index=0, limit=10):
        self._record("get_chart_artists", genre_id, index, limit)
        if self.fail:
            raise DeezerUnavailable("down")
        return {
            "data": [
                {"id": i, "name": f"Chart {i}", "picture_medium": "https://img/c.jpg"}
                for i in range(1, 101)
            ]
        }

    def get_playlist_tracks(self, playlist_id, index=0, limit=50):
        self._record("get_playlist_tracks", str(playlist_id), index, limit)
        if self.fail:
            raise DeezerUnavailable("down")
        base = 1000 + int(str(playlist_id)[-2:]) * 100
        return {
            "data": [
                {
                    "artist": {
                        "id": base + i,
                        "name": f"Country {playlist_id} {i}",
                        "picture_medium": "https://img/p.jpg",
                    }
                }
                for i in range(10)
            ]
        }

    def get_editorial_charts(self, genre_id, index=0, limit=10):
        self._record("get_editorial_charts", genre_id)
        if self.fail:
            raise DeezerUnavailable("down")
        base = 2000 + int(genre_id) * 10
        return {
            "albums": [
                {
                    "artist": {
                        "id": base + i,
                        "name": f"Genre {genre_id} A{i}",
                        "picture_medium": "https://img/e.jpg",
                    }
                }
                for i in range(5)
            ],
            "tracks": [
                {
                    "artist": {
                        "id": base + 100 + i,
                        "name": f"Genre {genre_id} T{i}",
                        "picture_medium": "https://img/e.jpg",
                    }
                }
                for i in range(5)
            ],
        }

    def search_artist(self, query, index=0, limit=25):
        self._record("search_artist", query, index, limit)
        if self.fail:
            raise DeezerUnavailable("down")
        return {
            "data": [
                {
                    "id": 900,
                    "name": "Daft Punk Tribute",
                    "picture_medium": "https://img/900.jpg",
                    "nb_fan": 5000,
                }
            ][:limit]
        }

    def get_artist(self, artist_id):
        self._record("get_artist", str(artist_id))
        if self.fail:
            raise DeezerUnavailable("down")
        return {
            "id": int(artist_id),
            "name": f"Artist {artist_id}",
            "picture_medium": "https://img/a.jpg",
        }

    def get_artist_related(self, artist_id, index=0, limit=20):
        self._record("get_artist_related", str(artist_id), index, limit)
        if self.fail:
            raise DeezerUnavailable("down")
        return {
            "data": [
                {
                    "id": 500 + i,
                    "name": f"Related {i}",
                    "picture_medium": f"https://img/r{i}.jpg",
                }
                for i in range(20)
            ]
        }


class FakeYT:
    def __init__(self):
        self.calls = []

    def get_charts(self, country="ZZ"):
        self.calls.append(country)
        return {
            "artists": [
                {
                    "title": f"YouTube {country} {i}",
                    "browseId": f"UC{country}{i:08d}",
                    "subscribers": "1.2M",
                    "thumbnails": [
                        {
                            "url": "https://yt3.googleusercontent.com/x=w60-h60-p-l90-rj",
                            "width": 60,
                        },
                        {
                            "url": "https://yt3.googleusercontent.com/x=w120-h120-p-l90-rj",
                            "width": 120,
                        },
                    ],
                    "rank": str(i + 1),
                }
                for i in range(40)
            ]
        }

    def search(self, query, filter=None, limit=20):
        self.calls.append(("search", query, filter, limit))
        return [
            {
                "title": "YouTube Search",
                "browseId": "UCsearch000000000000",
                "thumbnails": [],
            }
        ]


class OnboardingTestCase(unittest.TestCase):
    def setUp(self):
        self.__db = tempfile.mkstemp()
        self.__dir = tempfile.mkdtemp()
        self.archive = tempfile.mkdtemp()
        db_path = self.__db[1]
        cache_path = self.__dir

        class Config(DefaultConfig):
            TESTING = True

            def __init__(self):
                super().__init__()
                self.BASE = dict(self.BASE, database_uri="sqlite:///" + db_path)
                self.WEBAPP = dict(
                    self.WEBAPP,
                    cache_dir=cache_path,
                    mount_webui=True,
                    mount_api=True,
                    onboarding_inline=False,
                )

        self.app = create_application(Config())
        if "onboarding_api" not in self.app.blueprints:
            self.app.register_blueprint(onboarding_api, url_prefix="/api")
        self.fake = FakeApi()
        pool.reset_state()
        pool.set_public_client(self.fake)
        from neezer.webui import onboarding as routes

        routes.search_limiter.clear()
        routes.retry_limiter.clear()
        routes.like_limiter.clear()
        self.alice = UserManager.add("alice", "Alic3", admin=True)
        self.bob = UserManager.add("bob", "B0b")
        User.update(onboarding_step="artists").where(
            User.id.in_([self.alice.id, self.bob.id])
        ).execute()
        self.alice = User[self.alice.id]
        self.bob = User[self.bob.id]
        self.client = self.app.test_client()

    def tearDown(self):
        pool.reset_state()
        from neezer.webui import onboarding as routes

        routes.search_limiter.clear()
        routes.retry_limiter.clear()
        routes.like_limiter.clear()
        release_database()
        shutil.rmtree(self.__dir, ignore_errors=True)
        shutil.rmtree(self.archive, ignore_errors=True)
        os.close(self.__db[0])
        os.remove(self.__db[1])

    def _login(self, name="alice", password="Alic3"):
        response = self.client.post(
            "/api/login", json={"username": name, "password": password}
        )
        self.assertEqual(response.status_code, 200, response.get_json())
        return response

    def _install(self, cards, kind="deezer", built_at=None):
        pool.install_snapshot(self.app, kind, cards, built_at=built_at)

    def _local_artist(self, name, deezer_id=None, plays=0, owner=None):
        root = Folder.get_or_none(Folder.root == True)
        if root is None:
            root = Folder.create(root=True, name="Music", path=self.archive)
        artist = Artist.create(name=name, deezer_id=deezer_id)
        album = Album.create(name=f"Album {name}", artist=artist)
        path = os.path.join(self.archive, f"{uuid.uuid4()}.flac")
        with open(path, "wb") as handle:
            handle.write(b"\0")
        Track.create(
            disc=1,
            number=1,
            title=f"Track {name}",
            duration=1,
            album=album,
            artist=artist,
            bitrate=0,
            path=path,
            last_modification=1,
            root_folder=root,
            folder=root,
            owner=owner,
            play_count=plays,
        )
        return artist

    def _library(self, count=24, prefix="Library"):
        return [self._local_artist(f"{prefix} {i}", plays=i) for i in range(count)]

    def _like(self, ref):
        return self.client.post("/api/onboarding/like", json={"deezer_id": str(ref)})

    def _page(self, **params):
        query = "&".join(f"{key}={value}" for key, value in params.items())
        return self.client.get("/api/onboarding/artists" + (f"?{query}" if query else ""))


class DiversityTestCase(unittest.TestCase):
    def _rich_pool(self):
        cards = []
        for genre in range(8):
            for index in range(20):
                cards.append(
                    Card(
                        provider="deezer",
                        ref=str(genre * 100 + index),
                        name=f"G{genre} A{index}",
                        genres=(f"G{genre}",),
                        sources=("genre",),
                        source="genre",
                        rank=index + 1,
                    )
                )
        for country_index, country in enumerate(COUNTRIES):
            for index in range(20):
                cards.append(
                    Card(
                        provider="deezer",
                        ref=str(10000 + country_index * 100 + index),
                        name=f"{country} A{index}",
                        countries=(country,),
                        sources=("country",),
                        source="country",
                        rank=index + 1,
                    )
                )
        for index in range(100):
            cards.append(
                Card(
                    provider="deezer",
                    ref=str(20000 + index),
                    name=f"Chart {index}",
                    sources=("chart",),
                    source="chart",
                    rank=index + 1,
                )
            )
        return cards

    def test_diversify_is_deterministic_and_keeps_every_ref_once(self):
        cards = self._rich_pool()
        first = diversify(cards, "seed-a", limit=240)
        second = diversify(cards, "seed-a", limit=240)
        self.assertEqual([card.key for card in first], [card.key for card in second])
        self.assertEqual(len({card.key for card in first}), len(first))
        self.assertNotEqual(
            [card.key for card in first],
            [card.key for card in diversify(cards, "seed-b", limit=240)],
        )

    def test_first_page_covers_buckets_and_regions(self):
        cards = self._rich_pool()
        out = diversify(cards, "seed", limit=24)
        buckets = [pool_name(card) for card in out]
        regions = [region_name(card) for card in out]
        self.assertGreaterEqual(len(set(buckets[:8])), 8)
        self.assertGreaterEqual(len({b for b in buckets[:24] if b.startswith("genre:")}), 8)
        self.assertGreaterEqual(len({r for r in regions[:24] if r != "?"}), 3)

    def test_quotas_hold_on_a_rich_pool(self):
        cards = self._rich_pool()
        out = diversify(cards, "seed", limit=240)
        buckets = [pool_name(card) for card in out]
        regions = [region_name(card) for card in out]
        self.assertLessEqual(
            max(
                sum(1 for value in buckets[i : i + 12] if value == bucket)
                for bucket in set(buckets)
                for i in range(len(buckets) - 11)
            ),
            DIVERSITY["max_same_genre"],
        )
        self.assertLessEqual(
            max(
                sum(1 for value in buckets[i : i + 12] if value == "chart")
                for i in range(len(buckets) - 11)
            ),
            DIVERSITY["chart_max"],
        )
        real_regions = {value for value in regions if value != "?"}
        self.assertLessEqual(
            max(
                sum(1 for value in regions[i : i + 12] if value == region)
                for region in real_regions
                for i in range(len(regions) - 11)
            ),
            DIVERSITY["region_max"],
        )
        self.assertLessEqual(
            max(
                len(list(group))
                for _value, group in itertools.groupby(buckets)
            ),
            DIVERSITY["max_streak"],
        )

    def test_a_poor_pool_returns_every_card(self):
        cards = [
            Card("deezer", "1", "One", sources=("chart",), source="chart", rank=1),
            Card("deezer", "2", "Two", sources=("chart",), source="chart", rank=2),
            Card("deezer", "3", "Three", sources=("chart",), source="chart", rank=3),
        ]
        out = diversify(cards, "seed", limit=100)
        self.assertEqual([card.ref for card in out], ["1", "2", "3"])

    def test_homonyms_are_not_merged(self):
        cards = [
            Card("deezer", "1", "Same", sources=("country",), source="country", countries=("FR",), rank=1),
            Card("deezer", "2", "Same", sources=("country",), source="country", countries=("US",), rank=2),
        ]
        out = diversify(cards, "seed")
        self.assertEqual(len(out), 2)
        self.assertEqual({card.ref for card in out}, {"1", "2"})

    def test_pick_similar_keeps_the_provider_order_and_filters(self):
        candidates = [deezer_card(10 + i, f"Rel {i}", sources=("related",)) for i in range(6)]
        picked = pick_similar(
            candidates,
            exclude_refs=["10", "12"],
            exclude_names=["rel 3"],
            count=4,
        )
        self.assertEqual([card.ref for card in picked], ["11", "14", "15"])

    def test_pick_similar_caps_one_genre_to_three(self):
        candidates = [
            deezer_card(i, f"Rel {i}", genres=("Pop",), sources=("related",))
            for i in range(1, 6)
        ]
        candidates.append(deezer_card(9, "Other", genres=("Jazz",), sources=("related",)))
        picked = pick_similar(candidates, count=4)
        self.assertEqual(len(picked), 4)
        self.assertEqual(sum(1 for card in picked if "Pop" in card.genres), 3)
        self.assertEqual(picked[-1].ref, "9")

    def test_rank_search_prefers_exact_then_prefix_then_contains(self):
        remote = [
            deezer_card(1, "The Daft Punk Tribute", sources=("search",)),
            deezer_card(2, "Daft Punk", sources=("search",)),
            deezer_card(3, "Daft Punk Live", sources=("search",)),
        ]
        ranked = rank_search([], remote, "daft punk")
        self.assertEqual([card.ref for card in ranked], ["2", "3", "1"])

    def test_rank_search_keeps_homonyms_with_a_hint(self):
        local = [deezer_card(1, "Same", countries=("FR",), sources=("library",))]
        remote = [deezer_card(2, "Same", countries=("US",), sources=("search",))]
        ranked = rank_search(local, remote, "same")
        self.assertEqual(len(ranked), 2)
        self.assertEqual(ranked[0].hint, "France")
        self.assertEqual(ranked[1].hint, "États-Unis")

    def test_cursors(self):
        self.assertEqual(read_after(None), 0)
        self.assertEqual(read_after("24"), 24)
        self.assertIsNone(read_after("-1"))
        self.assertIsNone(read_after("601"))
        self.assertEqual(next_after(24, 30), "24")
        self.assertIsNone(next_after(30, 30))


def pool_name(card):
    from neezer.onboarding import _primary_bucket

    return _primary_bucket(card, collections.Counter())


def region_name(card):
    from neezer.onboarding import _region

    return _region(card, pool_name(card))


class PoolBuildTestCase(OnboardingTestCase):
    def test_cold_build_is_21_calls_and_has_the_nine_countries(self):
        snapshot = pool.build_now(self.app, "deezer", client=self.fake)
        self.assertEqual(len(self.fake.calls), 21)
        self.assertEqual(snapshot["error"], None)
        self.assertGreaterEqual(len(snapshot["cards"]), 80)
        countries = {country for card in snapshot["cards"] for country in card.countries}
        self.assertEqual(countries, set(COUNTRIES))
        names = [call[0] for call in self.fake.calls]
        self.assertEqual(names.count("get_genres"), 1)
        self.assertEqual(names.count("get_chart_artists"), 1)
        self.assertEqual(names.count("get_playlist_tracks"), 9)
        self.assertEqual(names.count("get_editorial_charts"), 10)

    def test_a_warm_pool_costs_no_call(self):
        self._install([deezer_card(i) for i in range(1, 101)])
        with mock.patch.object(self.fake, "get_genres", side_effect=AssertionError):
            cards, source, _count = pool.feed_cards(self.app, self.alice)
        self.assertEqual(source, "deezer")
        self.assertEqual(len(cards), 100)
        self.assertEqual(self.fake.calls, [])

    def test_a_failed_rebuild_keeps_the_last_good_pool(self):
        self._install([deezer_card(i) for i in range(1, 101)])
        old = pool.get_pool(self.app, "deezer")
        self.fake.fail = True
        pool.build_now(self.app, "deezer", client=self.fake)
        kept = pool.get_pool(self.app, "deezer")
        self.assertEqual(len(kept["cards"]), len(old["cards"]))
        self.assertEqual(pool.state_of(self.app, "deezer")["error"], "deezer-unreachable")

    def test_a_partial_pool_is_accepted_when_there_is_no_old_one(self):
        calls = {"n": 0}
        original = self.fake.get_playlist_tracks

        def fail_after_genres(*args, **kwargs):
            calls["n"] += 1
            raise DeezerUnavailable("down")

        self.fake.get_playlist_tracks = fail_after_genres
        snapshot = pool.build_now(self.app, "deezer", client=self.fake)
        self.assertGreaterEqual(len(snapshot["cards"]), 80)
        self.assertTrue(snapshot["partial"])
        self.assertEqual(snapshot["error"], "deezer-unreachable")
        self.assertGreaterEqual(calls["n"], 1)

    def test_the_build_stops_at_the_first_transport_failure(self):
        self.fake.fail = True
        snapshot = pool.build_now(self.app, "deezer", client=self.fake)
        self.assertEqual(snapshot["error"], "deezer-unreachable")
        self.assertEqual(len(self.fake.calls), 1)

    def test_youtube_build_uses_ten_charts_and_rewrites_portraits(self):
        fake = FakeYT()
        with mock.patch.object(yt, "installed", return_value=True):
            snapshot = pool.build_youtube(fake)
        self.assertEqual(snapshot["error"], None)
        self.assertEqual(len(fake.calls), 10)
        self.assertEqual(fake.calls[0], "ZZ")
        self.assertEqual(len(snapshot["cards"]), 400)
        self.assertTrue(snapshot["cards"][0].picture.endswith("=w250-h250-p-l90-rj"))
        self.assertTrue(snapshot["cards"][0].picture_small.endswith("=w60-h60-p-l90-rj"))
        countries = {
            country for card in snapshot["cards"] for country in card.countries
        }
        self.assertEqual(countries, set(COUNTRIES))

    def test_youtube_absent_disabled_and_held_are_named(self):
        with mock.patch.object(yt, "installed", return_value=False):
            self.assertEqual(pool.build_youtube()["error"], "absent")
        with mock.patch.object(yt, "installed", return_value=True), mock.patch.object(
            pool, "_meta", return_value="0"
        ):
            self.assertEqual(pool.build_youtube()["error"], "disabled")
        with mock.patch.object(yt, "installed", return_value=True), mock.patch.object(
            yt, "held", return_value=(True, 30, "refused")
        ):
            self.assertEqual(pool.build_youtube()["error"], "held")

    def test_the_feed_order_is_deezer_then_library_then_youtube(self):
        self._install([deezer_card(i) for i in range(1, 101)])
        cards, source, _count = pool.feed_cards(self.app, self.alice)
        self.assertEqual(source, "deezer")
        pool.install_snapshot(self.app, "deezer", [])
        self._library(24)
        cards, source, _count = pool.feed_cards(self.app, self.alice)
        self.assertEqual(source, "library")
        self.assertEqual(len(cards), 24)
        Track.delete().execute()
        Album.delete().execute()
        Artist.delete().execute()
        pool.reset_state()
        pool.set_public_client(self.fake)
        self._local_artist("Only Local")
        self._install(
            [
                youtube_card(f"UC{i:022d}", f"YouTube {i}", countries=("FR",))
                for i in range(40)
            ],
            kind="youtube",
        )
        with mock.patch.object(yt, "installed", return_value=True), mock.patch.object(
            pool.access, "can", return_value=True
        ):
            cards, source, count = pool.feed_cards(self.app, self.alice)
        self.assertEqual(source, "youtube")
        self.assertEqual(count, 1)
        self.assertEqual(len(cards), 41)

    def test_youtube_is_only_considered_when_the_library_is_short(self):
        self.app.config["WEBAPP"]["onboarding_inline"] = True
        calls = []

        def record(app, kind="deezer", force=False):
            calls.append(kind)
            return True

        with mock.patch.object(pool, "ensure_build", side_effect=record):
            self._install([deezer_card(i) for i in range(1, 101)])
            pool.ensure_feed(self.app, self.alice, library_count=0)
            self.assertEqual(calls, [])
            self._install([], kind="deezer")
            self.app.deezer = None
            pool.ensure_feed(self.app, self.alice, library_count=30)
            self.assertEqual(calls, [])
            with mock.patch.object(yt, "installed", return_value=True), mock.patch.object(
                pool.access, "can", return_value=True
            ):
                pool.ensure_feed(self.app, self.alice, library_count=5)
            self.assertEqual(calls, ["youtube"])

    def test_similar_links_are_written_once_and_read_from_cache(self):
        self._install([deezer_card(5, "Parent")] + [deezer_card(i) for i in range(1, 100)])
        pool.run_related_now(self.app, "5")
        first = self.fake.calls.count(("get_artist_related", ("5", 0, 20)))
        self.assertEqual(first, 1)
        links, fresh, _stale = pool.related_links(self.app, "5")
        self.assertTrue(fresh)
        self.assertEqual(len(links), 20)
        pool.run_related_now(self.app, "5")
        self.assertEqual(
            self.fake.calls.count(("get_artist_related", ("5", 0, 20))), 2
        )


class OnboardingApiTestCase(OnboardingTestCase):
    def test_state_of_a_new_account(self):
        self._login()
        payload = self.client.get("/api/onboarding").get_json()
        self.assertEqual(payload["step"], "artists")
        self.assertEqual(payload["liked"], 0)
        self.assertEqual(payload["min_artists"], 1)
        self.assertEqual(payload["diversity"]["window"], 12)
        self.assertIn("account", payload)
        self.assertEqual(payload["account"]["can_save_imports"], True)
        self.assertIn("youtube", payload)

    def test_a_page_is_zero_provider_calls_and_paginates(self):
        self._install([deezer_card(i, f"Artist {i}") for i in range(1, 101)])
        self._login()
        first = self._page(limit=24).get_json()
        self.assertEqual(first["source"], "deezer")
        self.assertEqual(len(first["items"]), 24)
        self.assertEqual(first["next"], "24")
        self.assertEqual(self.fake.calls, [])
        second = self._page(after=24, limit=24).get_json()
        self.assertEqual(len(second["items"]), 24)
        self.assertNotEqual(first["items"][0]["deezer_id"], second["items"][0]["deezer_id"])
        self.assertEqual(second["next"], "48")
        self.assertEqual(self.fake.calls, [])

    def test_the_page_uses_additive_fields_and_marks_liked_artists(self):
        self._install([deezer_card(i) for i in range(1, 31)])
        self._login()
        before = self._page(limit=24).get_json()
        liked = before["items"][0]["deezer_id"]
        self.assertEqual(self._like(liked).status_code, 200)
        page = self._page(limit=24).get_json()
        self.assertEqual(len(page["items"]), 24)
        self.assertIn(liked, {item["deezer_id"] for item in page["items"]})
        marked = next(item for item in page["items"] if item["deezer_id"] == liked)
        self.assertTrue(marked["liked"])
        self.assertEqual(page["next"], "24")
        next_page = self._page(after=24, limit=24).get_json()
        cards, _source, _count = pool.feed_cards(self.app, self.alice)
        build = pool.state_of(self.app, "deezer")["build"]
        ordered = diversify(cards, seed_for(self.alice.id, build), limit=600)
        self.assertEqual(next_page["items"][0]["deezer_id"], ordered[24].api_id)
        item = page["items"][0]
        self.assertIn("provider", item)
        self.assertIn("openable", item)
        self.assertIn("hint", item)

    def test_a_page_costs_at_most_three_statements(self):
        self._install([deezer_card(i) for i in range(1, 101)])
        self._login()
        with count_statements() as counts:
            self._page(limit=24)
        self.assertLessEqual(sum(counts.values()), 3, counts)

    def test_state_costs_at_most_four_statements(self):
        self._install([deezer_card(i) for i in range(1, 101)])
        self._login()
        with count_statements() as counts:
            self.client.get("/api/onboarding")
        self.assertLessEqual(sum(counts.values()), 4, counts)

    def test_the_library_paints_when_deezer_is_down(self):
        self._library(24)
        self.fake.fail = True
        self._login()
        page = self._page(limit=24).get_json()
        self.assertEqual(page["source"], "library")
        self.assertTrue(page["degraded"])
        self.assertEqual(len(page["items"]), 24)
        self.assertEqual(self.fake.calls, [])

    def test_like_is_idempotent_and_writes_no_track(self):
        self._install([deezer_card(5, "Real Artist")])
        self._login()
        with mock.patch("neezer.deezer.youtube.download_audio") as download, mock.patch(
            "neezer.deezer.archive.ensure_archived"
        ) as archived, mock.patch(
            "neezer.deezer.backfill.archive_entity"
        ) as backfilled, mock.patch(
            "neezer.deezer.prefetch.DeezerPrefetcher.download_ids"
        ) as prefetched, mock.patch(
            "neezer.webui.onboarding.discover.invalidate"
        ) as invalidated:
            first = self._like(5)
            second = self._like(5)
            first = self._like(5)
            second = self._like(5)
        self.assertEqual(first.status_code, 200, first.get_json())
        self.assertTrue(first.get_json()["liked"])
        self.assertEqual(second.status_code, 200)
        self.assertEqual(Artist.select().count(), 1)
        self.assertEqual(StarredArtist.select().count(), 1)
        self.assertEqual(Track.select().count(), 0)
        self.assertEqual(Album.select().count(), 0)
        self.assertEqual(download.call_count, 0)
        self.assertEqual(archived.call_count, 0)
        self.assertEqual(backfilled.call_count, 0)
        self.assertEqual(prefetched.call_count, 0)
        self.assertGreaterEqual(invalidated.call_count, 2)
        self.assertEqual(self.fake.calls, [])

    def test_a_like_does_not_rename_a_shared_artist(self):
        row = Artist.create(name="Real", deezer_id="5")
        self._install([deezer_card(5, "Pool Name")])
        self._login()
        response = self.client.post(
            "/api/onboarding/like", json={"deezer_id": "5", "name": "Injected"}
        )
        self.assertEqual(response.status_code, 200, response.get_json())
        self.assertEqual(Artist[row.id].name, "Real")
        self.assertEqual(response.get_json()["artist"]["name"], "Real")

    def test_unlike_is_idempotent_and_keeps_the_artist_row(self):
        self._install([deezer_card(5)])
        self._login()
        self._like(5)
        first = self.client.post("/api/onboarding/unlike", json={"deezer_id": "5"})
        second = self.client.post("/api/onboarding/unlike", json={"deezer_id": "5"})
        self.assertEqual(first.status_code, 200)
        self.assertEqual(first.get_json(), {"liked": False})
        self.assertEqual(second.status_code, 200)
        self.assertEqual(StarredArtist.select().count(), 0)
        self.assertEqual(Artist.select().count(), 1)

    def test_a_like_unmutes_the_artist(self):
        self._install([deezer_card(5)])
        self._login()
        ArtistMute.create(user=self.alice, provider="deezer", ref="5")
        self._like(5)
        self.assertEqual(ArtistMute.select().count(), 0)

    def test_a_cold_related_cache_answers_pending_and_queues_once(self):
        self._install([deezer_card(5)])
        self._login()
        first = self._like(5).get_json()["related"]
        second = self._like(5).get_json()["related"]
        self.assertEqual(first["state"], "pending")
        self.assertEqual(second["state"], "pending")
        self.assertEqual(len(pool._related_queue), 1)
        self.assertEqual(self.fake.calls.count(("get_artist_related", ("5", 0, 20))), 0)

    def test_related_is_inserted_after_with_the_provider_order(self):
        self._install([deezer_card(5, "Parent")])
        rows = [
            ArtistLink.create(
                provider="deezer",
                artist_ref="5",
                other_ref=str(500 + index),
                other_name=f"Related {index}",
                other_picture=f"https://img/r{index}.jpg",
                score=1.0,
                pos=index + 1,
                fetched=now(),
            )
            for index in range(6)
        ]
        self._login()
        related = self._like(5).get_json()["related"]
        self.assertEqual(related["state"], "ready")
        self.assertEqual(
            [item["deezer_id"] for item in related["items"]],
            ["500", "501", "502", "503"],
        )
        self.assertEqual(related["items"][0]["parent"], "5")
        self.assertEqual(related["items"][0]["via"], "related")

    def test_the_related_route_requires_a_star_and_then_reads_the_cache(self):
        self._install([deezer_card(5)])
        self._login()
        self.assertEqual(
            self.client.get("/api/onboarding/related/5").status_code, 404
        )
        self._like(5)
        pool.run_related_now(self.app, "5")
        before = len(self.fake.calls)
        response = self.client.get("/api/onboarding/related/5")
        self.assertEqual(response.status_code, 200, response.get_json())
        payload = response.get_json()
        self.assertEqual(payload["state"], "ready")
        self.assertEqual(len(payload["items"]), 4)
        self.assertEqual(len(self.fake.calls), before)

    def test_youtube_like_uses_a_deterministic_uuid_and_an_entity_ref(self):
        ref = "UC" + "a" * 22
        card = youtube_card(ref, "YouTube Artist", countries=("FR",))
        self._install([card], kind="youtube")
        self._login()
        response = self.client.post(
            "/api/onboarding/like", json={"deezer_id": card.entity_id, "name": "Faux"}
        )
        self.assertEqual(response.status_code, 200, response.get_json())
        self.assertEqual(response.get_json()["artist"]["name"], "YouTube Artist")
        self.assertTrue(
            response.get_json()["artist"]["picture"].startswith(
                "/api/onboarding/picture/"
            )
        )
        self.assertEqual(response.get_json()["related"]["state"], "none")
        row = Artist.get()
        self.assertEqual(row.id, ids.youtube_artist_uuid(ref))
        self.assertIsNone(row.deezer_id)
        self.assertEqual(row.name, "YouTube Artist")
        self.assertEqual(EntityRef.select().count(), 1)
        self.assertEqual(StarredArtist.select().count(), 1)
        self.assertEqual(Track.select().count(), 0)
        self._like(card.entity_id)
        self.assertEqual(Artist.select().count(), 1)
        self.assertEqual(StarredArtist.select().count(), 1)

    def test_homonyms_are_not_merged(self):
        self._install(
            [deezer_card(1, "Same"), deezer_card(2, "Same")]
        )
        self._login()
        page = self._page(limit=24).get_json()
        self.assertEqual(len(page["items"]), 2)
        self.assertTrue(all(item["hint"] for item in page["items"]))
        self._like(1)
        page = self._page(limit=24).get_json()
        self.assertEqual([item["deezer_id"] for item in page["items"]], ["1", "2"])
        self.assertTrue(
            next(item for item in page["items"] if item["deezer_id"] == "1")["liked"]
        )
        self.assertEqual(Artist.select().count(), 1)

    def test_search_local_first_then_one_provider_call(self):
        self._local_artist("Daft Punk", deezer_id="27")
        self._install([deezer_card(900, "Daft Punk Tribute")])
        self._login()
        payload = self.client.get("/api/onboarding/search?q=daft%20punk").get_json()
        self.assertEqual(payload["query"], "daft punk")
        self.assertEqual(payload["local"], 2)
        self.assertEqual(payload["items"][0]["deezer_id"], "27")
        self.assertIn("900", {item["deezer_id"] for item in payload["items"]})
        self.assertEqual(
            [call[0] for call in self.fake.calls].count("search_artist"), 1
        )

    def test_search_empty_and_one_character_make_no_call(self):
        self._local_artist("Daft Punk", deezer_id="27")
        self._login()
        empty = self.client.get("/api/onboarding/search?q=").get_json()
        self.assertEqual(empty["items"], [])
        one = self.client.get("/api/onboarding/search?q=d").get_json()
        self.assertTrue(one["local_only"])
        self.assertEqual(one["items"][0]["deezer_id"], "27")
        self.assertEqual(self.fake.calls, [])

    def test_search_falls_back_to_the_library_when_deezer_is_down(self):
        self._local_artist("Daft Punk", deezer_id="27")
        self._install([deezer_card(900, "Daft Punk Tribute")])
        self.fake.fail = True
        self._login()
        payload = self.client.get("/api/onboarding/search?q=daft%20punk").get_json()
        self.assertTrue(payload["partial"])
        self.assertEqual(payload["error"], "deezer-unreachable")
        self.assertEqual(payload["items"][0]["deezer_id"], "27")

    def test_the_artists_route_accepts_q_for_the_client_contract(self):
        self._local_artist("Daft Punk", deezer_id="27")
        self._install([deezer_card(900, "Daft Punk Tribute")])
        self._login()
        payload = self.client.get("/api/onboarding/artists?q=daft").get_json()
        self.assertEqual(payload["source"], "search")
        self.assertIsNone(payload["next"])
        self.assertEqual(payload["items"][0]["deezer_id"], "27")

    def test_done_needs_one_artist_then_moves_to_account_and_done(self):
        self._install([deezer_card(5)])
        self._login()
        refused = self.client.post("/api/onboarding/artists/done", json={})
        self.assertEqual(refused.status_code, 409)
        self.assertEqual(refused.get_json()["code"], "min_artists")
        self.assertEqual(
            self.client.post("/api/onboarding/account/done", json={}).status_code, 409
        )
        self._like(5)
        moved = self.client.post("/api/onboarding/artists/done", json={})
        self.assertEqual(moved.status_code, 200)
        self.assertEqual(moved.get_json(), {"step": "account"})
        self.assertEqual(
            self.client.post("/api/onboarding/artists/done", json={}).get_json()["step"],
            "account",
        )
        finished = self.client.post("/api/onboarding/account/done", json={})
        self.assertEqual(finished.status_code, 200)
        self.assertEqual(finished.get_json()["step"], "done")
        self.assertEqual(finished.get_json()["user"]["onboarding"], "done")
        blocked = self._like(5)
        self.assertEqual(blocked.status_code, 409)
        self.assertEqual(blocked.get_json()["code"], "onboarding_done")

    def test_skip_is_only_available_when_every_source_failed(self):
        self._login()
        page = self._page().get_json()
        self.assertEqual(page["state"], "unavailable")
        skipped = self.client.post(
            "/api/onboarding/artists/done", json={"skip": True}
        )
        self.assertEqual(skipped.status_code, 200, skipped.get_json())
        self.assertTrue(skipped.get_json()["skipped"])
        self.assertEqual(User[self.alice.id].onboarding_step, "done")

    def test_skip_is_refused_when_artists_are_available(self):
        self._install([deezer_card(5)])
        self._login()
        refused = self.client.post(
            "/api/onboarding/artists/done", json={"skip": True}
        )
        self.assertEqual(refused.status_code, 409)
        self.assertEqual(refused.get_json()["code"], "artists_available")

    def test_401_403_404_400(self):
        self.assertEqual(self.client.get("/api/onboarding").status_code, 401)
        self._login()
        cross = self.client.post(
            "/api/onboarding/like",
            json={"deezer_id": "5"},
            headers={"Origin": "https://evil.example"},
        )
        self.assertEqual(cross.status_code, 403)
        site = self.client.post(
            "/api/onboarding/like",
            json={"deezer_id": "5"},
            headers={"Sec-Fetch-Site": "cross-site"},
        )
        self.assertEqual(site.status_code, 403)
        self.assertEqual(
            self.client.get("/api/onboarding/related/999").status_code, 404
        )
        self.assertEqual(
            self.client.post(
                "/api/onboarding/like", json={"deezer_id": "../x"}
            ).status_code,
            400,
        )
        self.assertEqual(
            self.client.post(
                "/api/onboarding/unlike", json={"deezer_id": "-1"}
            ).status_code,
            400,
        )

    def test_search_budget_returns_429_with_retry_after(self):
        self._install([deezer_card(5)])
        self._login()
        for index in range(40):
            response = self.client.get(f"/api/onboarding/search?q=x{index}")
            self.assertEqual(response.status_code, 200, response.get_json())
        blocked = self.client.get("/api/onboarding/search?q=x40")
        self.assertEqual(blocked.status_code, 429)
        self.assertTrue(blocked.headers.get("Retry-After"))

    def test_feed_retry_is_bounded_and_409_after_done(self):
        self._login()
        first = self.client.post("/api/onboarding/feed/retry")
        second = self.client.post("/api/onboarding/feed/retry")
        self.assertIn(first.status_code, (200, 202))
        self.assertEqual(second.status_code, 429)
        User.update(onboarding_step="done").where(User.id == self.alice.id).execute()
        self.alice = User[self.alice.id]
        self.assertEqual(
            self.client.post("/api/onboarding/feed/retry").status_code, 409
        )

    def test_an_oversized_body_is_413(self):
        self._login()
        response = self.client.post(
            "/api/onboarding/like",
            data=b"{" + b" " * 9000 + b"}",
            content_type="application/json",
        )
        self.assertEqual(response.status_code, 413)

    def test_picture_route_serves_the_cached_bytes_without_google(self):
        ref = "UC" + "b" * 22
        card = youtube_card(ref, "Pic")
        self._install([card], kind="youtube")
        self._login()
        directory = os.path.join(
            self.app.config["WEBAPP"]["cache_dir"], "onboarding", "pictures"
        )
        os.makedirs(directory, exist_ok=True)
        digest = hashlib.sha1(f"{ref}:250".encode("utf-8")).hexdigest()
        path = os.path.join(directory, digest + ".jpg")
        with open(path, "wb") as handle:
            handle.write(b"\xff\xd8\xff\xe0" + b"0" * 64)
        with mock.patch("neezer.deezer.onboarding_pool.requests.get") as http:
            response = self.client.get(f"/api/onboarding/picture/{card.entity_id}")
        self.assertEqual(response.status_code, 200)
        self.assertEqual(response.mimetype, "image/jpeg")
        self.assertEqual(http.call_count, 0)
        response.close()
        self.assertEqual(
            self.client.get(
                "/api/onboarding/picture/00000000-0000-0000-0000-000000000001"
            ).status_code,
            404,
        )

    def test_a_starred_artist_survives_prune_and_leaves_after_unlike(self):
        self._install([deezer_card(5, "Kept")])
        self._login()
        self._like(5)
        self.assertEqual(Artist.prune(), 0)
        self.assertEqual(Artist.select().count(), 1)
        self.assertEqual(StarredArtist.select().count(), 1)
        self.client.post("/api/onboarding/unlike", json={"deezer_id": "5"})
        Artist.prune()
        self.assertEqual(Artist.select().count(), 0)

    def test_a_like_costs_at_most_five_statements_when_warm(self):
        self._install([deezer_card(5)])
        self._login()
        self._like(5)
        with count_statements() as counts:
            response = self._like(5)
        self.assertEqual(response.status_code, 200)
        self.assertLessEqual(sum(counts.values()), 5, counts)


if __name__ == "__main__":
    unittest.main()
