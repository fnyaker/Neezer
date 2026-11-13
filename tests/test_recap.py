# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""The Récap's own reads: ``GET /api/recap/years`` and ``GET /api/recap/<year>``.

A hand-built log, on two accounts, with a real genre tree: the year boundary is
checked on both sides of midnight, an empty year and a thin one answer 200 with
their real numbers, the tops and the streak are recomputed by hand, and the
genre card is refused when the measured verdicts do not cover enough listens.
The cost tests count STATEMENTS (1 for the year list, 4 for a year, 1 for an
empty one, plus the one session lookup every ``/api`` route pays) and prove the
aggregates are cached while the track payloads are not. No provider is called:
the detach guard covers it at runtime, and the module's own source is scanned.
"""

import datetime
import json
import pathlib
import unittest
from unittest import mock

from neezer import db as dbmod
from neezer import genretree, recap
from neezer.db import Album, Artist, ListenEvent, Track, TrackAnalysis, User
from neezer.managers.user import UserManager

from .test_deezer import count_statements
from .test_detach_a import _AppCase

NOW = dbmod.now()
YEAR = NOW.year
LAST_YEAR = YEAR - 1
EMPTY_YEAR = YEAR - 2

#: The order the server promises (§2.3 of the plan), share excluded.
CARD_ORDER = [
    "year",
    "minutes",
    "artists",
    "tracks",
    "albums",
    "genres",
    "streak",
    "firstlast",
]


class _RecapCase(_AppCase):
    def setUp(self):
        super().setUp()
        from neezer.deezer import library

        self.root = library.get_root_folder(self.archive)
        self.user = User.get(User.name == "alice")
        UserManager.add("bob", "B0bpass", admin=False)
        self.bob = User.get(User.name == "bob")
        self.artists = {}
        self.albums = {}
        self.tracks = {}
        self._make("Alpha", "100", "Alpha LP", "101", ["A1", "A2", "A3"])
        self._make("Beta", "200", "Beta LP", "201", ["B1", "B2"])
        # The genre tree is a process-wide cache (neezer/genretree.py): warm it,
        # so a statement count below is the request's own work and not the
        # tree's first load.
        genretree.get()
        recap.invalidate()

    def tearDown(self):
        recap.invalidate()
        super().tearDown()

    # -- fixtures -------------------------------------------------------------

    def _make(self, artist_name, artist_ref, album_name, album_ref, titles):
        artist = Artist.create(name=artist_name, deezer_id=artist_ref)
        self.artists[artist_name] = artist
        self._add_album(artist_name, album_name, album_ref, titles)

    def _add_album(self, artist_name, album_name, album_ref, titles):
        """Another album for an artist already in the fixture."""
        artist = self.artists[artist_name]
        album = Album.create(name=album_name, artist=artist, deezer_id=album_ref)
        self.albums[album_name] = album
        for i, title in enumerate(titles):
            track = Track.create(
                disc=1,
                number=i + 1,
                title=title,
                duration=210,
                album=album,
                artist=artist,
                bitrate=0,
                path=f"{self.archive}/{artist_name}-{album_name}-{title}.flac",
                last_modification=1,
                root_folder=self.root,
                folder=self.root,
            )
            self.tracks[title] = track
        return album

    def _event(self, track, when, listened=180, outcome=1, user=None):
        return ListenEvent.create(
            user=user or self.user,
            track=track,
            at=when,
            listened=listened,
            outcome=outcome,
            source="web",
        )

    def _analyse(self, track, style, confidence=0.8):
        return TrackAnalysis.create(track=track, style=style, style_confidence=confidence)

    # -- requests -------------------------------------------------------------

    def _years(self, client=None, query=""):
        return (client or self.client).get("/api/recap/years" + query)

    def _recap(self, year, client=None, query=""):
        return (client or self.client).get(f"/api/recap/{year}" + query)

    def _bob_client(self):
        client = self.app.test_client()
        self.assertEqual(
            client.post("/api/login", json={"username": "bob", "password": "B0bpass"}).status_code,
            200,
        )
        return client


class BoundaryTestCase(_RecapCase):
    def test_year_is_server_local_and_half_open(self):
        """An event at 23:59:59 on 31 December is the old year; the one at
        00:00:00 on 1 January is the new one — never both, never neither."""
        self._event(
            self.tracks["A1"], datetime.datetime(LAST_YEAR, 12, 31, 23, 59, 59), listened=200
        )
        self._event(
            self.tracks["A1"], datetime.datetime(YEAR, 1, 1, 0, 0, 0), listened=200
        )

        old = self._recap(LAST_YEAR).get_json()
        self.assertEqual(old["totals"]["events"], 1)
        self.assertEqual(old["first"]["at"], f"{LAST_YEAR}-12-31T23:59:59")
        self.assertEqual(old["last"]["at"], f"{LAST_YEAR}-12-31T23:59:59")

        new = self._recap(YEAR).get_json()
        self.assertEqual(new["totals"]["events"], 1)
        self.assertEqual(new["first"]["at"], f"{YEAR}-01-01T00:00:00")
        self.assertEqual(new["last"]["at"], f"{YEAR}-01-01T00:00:00")

    def test_partial_current_year_end_is_today(self):
        current = self._recap(YEAR).get_json()
        self.assertTrue(current["partial"])
        self.assertEqual(current["period"]["from"], f"{YEAR}-01-01")
        self.assertEqual(current["period"]["to"], dbmod.now().date().isoformat())
        self.assertEqual(current["timezone"], "server-local")

        finished = self._recap(LAST_YEAR).get_json()
        self.assertFalse(finished["partial"])
        self.assertEqual(finished["period"]["from"], f"{LAST_YEAR}-01-01")
        self.assertEqual(finished["period"]["to"], f"{LAST_YEAR}-12-31")


class EmptyAndThinTestCase(_RecapCase):
    def test_empty_year_is_200_with_zero_totals(self):
        response = self._recap(EMPTY_YEAR)
        self.assertEqual(response.status_code, 200)
        data = response.get_json()
        self.assertEqual(data["year"], EMPTY_YEAR)
        self.assertFalse(data["enough"])
        self.assertEqual(data["totals"]["events"], 0)
        self.assertEqual(data["totals"]["minutes"], 0)
        self.assertEqual(data["cards"], [])
        self.assertIsNone(data["streak"])
        self.assertIsNone(data["biggest_day"])
        self.assertIsNone(data["first"])
        self.assertIsNone(data["last"])
        self.assertEqual([m["month"] for m in data["months"]], list(range(1, 13)))
        self.assertEqual({m["listens"] for m in data["months"]}, {0})
        self.assertEqual(data["tops"]["tracks"], [])
        self.assertEqual(data["tops"]["artists"], [])
        self.assertEqual(data["tops"]["albums"], [])
        self.assertFalse(data["tops"]["genres"]["available"])

    def test_thin_year_reports_its_numbers_and_says_not_enough(self):
        """5 x 144 s = 720 s = 12 min: under both thresholds, but the real
        totals are served anyway — the screen has to be able to say them."""
        at = datetime.datetime(LAST_YEAR, 4, 1, 9, 0, 0)
        for i in range(5):
            self._event(
                self.tracks["A1"], at + datetime.timedelta(minutes=i), listened=144
            )
        data = self._recap(LAST_YEAR).get_json()
        self.assertFalse(data["enough"])
        self.assertEqual(data["cards"], [])
        self.assertEqual(data["totals"]["events"], 5)
        self.assertEqual(data["totals"]["listens"], 5)
        self.assertEqual(data["totals"]["minutes"], 12)

    def test_year_with_only_skips_is_not_empty(self):
        """Skips are real events: the year exists, with zero listens. Saying
        "aucune écoute" is right; saying "rien" would be wrong — and 33
        minutes of skipped time does NOT make it "ton année": `enough` needs
        one listen (plan §2.4)."""
        at = datetime.datetime(LAST_YEAR, 5, 2, 10, 0, 0)
        for i in range(70):
            self._event(
                self.tracks["B1"],
                at + datetime.timedelta(minutes=i),
                listened=29,
                outcome=-1,
            )
        data = self._recap(LAST_YEAR).get_json()
        self.assertEqual(data["totals"]["events"], 70)
        self.assertEqual(data["totals"]["listens"], 0)
        self.assertEqual(data["totals"]["skips"], 70)
        self.assertEqual(data["totals"]["neutrals"], 0)
        self.assertEqual(data["totals"]["seconds"], 2030)
        self.assertEqual(data["totals"]["minutes"], 33)
        self.assertFalse(data["enough"])
        self.assertEqual(data["cards"], [])
        self.assertIsNone(data["first"])
        self.assertIsNone(data["streak"])
        self.assertEqual(data["tops"]["tracks"], [])

        # The year list applies the same rule.
        chip = self._years().get_json()["years"][0]
        self.assertEqual(chip["year"], LAST_YEAR)
        self.assertFalse(chip["enough"])


class TotalsTestCase(_RecapCase):
    def _hand_log(self):
        """6 events: 3 listens, 2 skips, 1 neutral, 679 seconds over 3 days."""
        a1, a2, b1, b2 = (
            self.tracks["A1"],
            self.tracks["A2"],
            self.tracks["B1"],
            self.tracks["B2"],
        )
        self._event(a1, datetime.datetime(LAST_YEAR, 3, 1, 8, 0, 0), listened=200)
        self._event(b1, datetime.datetime(LAST_YEAR, 3, 1, 8, 5, 0), listened=5, outcome=-1)
        self._event(a2, datetime.datetime(LAST_YEAR, 3, 1, 8, 10, 0), listened=70, outcome=0)
        self._event(a1, datetime.datetime(LAST_YEAR, 3, 2, 9, 0, 0), listened=200)
        self._event(a1, datetime.datetime(LAST_YEAR, 3, 2, 9, 10, 0), listened=200)
        self._event(b2, datetime.datetime(LAST_YEAR, 3, 3, 9, 0, 0), listened=4, outcome=-1)

    def test_totals_match_a_hand_computed_log(self):
        self._hand_log()
        data = self._recap(LAST_YEAR).get_json()
        totals = data["totals"]
        self.assertEqual(totals["events"], 6)
        self.assertEqual(totals["listens"], 3)
        self.assertEqual(totals["skips"], 2)
        self.assertEqual(totals["neutrals"], 1)
        # 200 + 5 + 70 + 200 + 200 + 4 = 679 s, floored to the minute: 11.
        self.assertEqual(totals["seconds"], 679)
        self.assertEqual(totals["minutes"], 11)
        self.assertEqual(totals["days_active"], 2)
        self.assertEqual(totals["first_at"], f"{LAST_YEAR}-03-01T08:00:00")
        self.assertEqual(totals["last_at"], f"{LAST_YEAR}-03-02T09:10:00")
        self.assertEqual(
            data["biggest_day"],
            {"date": f"{LAST_YEAR}-03-02", "listens": 2, "seconds": 400},
        )
        # The month buckets hold the same truth, and only March is non-zero.
        self.assertEqual(data["months"][2], {"month": 3, "listens": 3, "skips": 2, "seconds": 679})
        self.assertEqual(sum(m["seconds"] for m in data["months"]), 679)
        self.assertEqual(data["first"]["track"]["title"], "A1")
        self.assertEqual(data["last"]["track"]["title"], "A1")

    def test_biggest_day_tie_goes_to_more_seconds_then_the_earlier_day(self):
        a1 = self.tracks["A1"]
        # Day one: 2 listens, 400 s. Day two: 2 listens, 600 s -> day two.
        self._event(a1, datetime.datetime(LAST_YEAR, 7, 1, 8, 0, 0), listened=200)
        self._event(a1, datetime.datetime(LAST_YEAR, 7, 1, 8, 5, 0), listened=200)
        self._event(a1, datetime.datetime(LAST_YEAR, 7, 2, 8, 0, 0), listened=300)
        self._event(a1, datetime.datetime(LAST_YEAR, 7, 2, 8, 5, 0), listened=300)
        data = self._recap(LAST_YEAR).get_json()
        self.assertEqual(data["biggest_day"]["date"], f"{LAST_YEAR}-07-02")
        self.assertEqual(data["biggest_day"]["seconds"], 600)

        # Same listens and same seconds on two days: the earlier one wins.
        TrackAnalysis.delete().execute()
        ListenEvent.delete().execute()
        recap.invalidate()
        self._event(a1, datetime.datetime(LAST_YEAR, 7, 3, 8, 0, 0), listened=250)
        self._event(a1, datetime.datetime(LAST_YEAR, 7, 4, 8, 0, 0), listened=250)
        data = self._recap(LAST_YEAR).get_json()
        self.assertEqual(data["biggest_day"]["date"], f"{LAST_YEAR}-07-03")

    def test_streak_counts_consecutive_days_and_clamps_to_the_year(self):
        at = self.tracks["A1"]
        self._event(at, datetime.datetime(LAST_YEAR, 12, 30, 9, 0, 0), listened=200)
        self._event(at, datetime.datetime(LAST_YEAR, 12, 31, 9, 0, 0), listened=200)
        self._event(at, datetime.datetime(YEAR, 1, 1, 9, 0, 0), listened=200)
        self._event(at, datetime.datetime(YEAR, 1, 2, 9, 0, 0), listened=200)

        old = self._recap(LAST_YEAR).get_json()["streak"]
        new = self._recap(YEAR).get_json()["streak"]
        # Clamped, not merged across the boundary: 2 + 2, never 4.
        self.assertEqual(
            old, {"days": 2, "from": f"{LAST_YEAR}-12-30", "to": f"{LAST_YEAR}-12-31"}
        )
        self.assertEqual(new, {"days": 2, "from": f"{YEAR}-01-01", "to": f"{YEAR}-01-02"})

        # A third day turns it into a card.
        self._event(at, datetime.datetime(YEAR, 1, 3, 9, 0, 0), listened=200)
        recap.invalidate()
        new = self._recap(YEAR).get_json()["streak"]
        self.assertEqual(new, {"days": 3, "from": f"{YEAR}-01-01", "to": f"{YEAR}-01-03"})


class TopsTestCase(_RecapCase):
    def test_top_tracks_are_by_listens_then_seconds_then_id(self):
        at = datetime.datetime(LAST_YEAR, 6, 1, 8, 0, 0)
        counters = {"A1": 3, "A2": 3, "B1": 2, "B2": 2, "A3": 1}
        seconds = {"A1": 200, "A2": 100, "B1": 100, "B2": 100, "A3": 150}
        for title, plays in counters.items():
            for i in range(plays):
                self._event(
                    self.tracks[title],
                    at + datetime.timedelta(minutes=i),
                    listened=seconds[title],
                )

        data = self._recap(LAST_YEAR).get_json()
        rows = data["tops"]["tracks"]
        # 5 rows maximum: the five titles with at least one listen.
        self.assertEqual(len(rows), 5)
        self.assertEqual(len({row["track"]["id"] for row in rows}), 5)
        titles = [row["track"]["title"] for row in rows]
        self.assertEqual(titles[:2], ["A1", "A2"])
        self.assertEqual(sorted(titles[2:4]), ["B1", "B2"])
        self.assertEqual(titles[4], "A3")
        self.assertEqual(rows[0]["listens"], 3)
        self.assertEqual(rows[0]["seconds"], 600)
        # B1 and B2 tie on listens and seconds: the smaller id comes first.
        self.assertEqual(
            titles[2:4],
            sorted(["B1", "B2"], key=lambda title: str(self.tracks[title].id)),
        )

    def test_top_artists_and_albums_sum_their_tracks(self):
        self._add_album("Alpha", "Alpha Second", "102", ["A4"])
        at = datetime.datetime(LAST_YEAR, 8, 1, 8, 0, 0)
        self._event(self.tracks["A1"], at, listened=200)
        for i in range(3):
            self._event(self.tracks["A4"], at + datetime.timedelta(minutes=10 + i), listened=200)
        for i in range(5):
            self._event(self.tracks["B1"], at + datetime.timedelta(minutes=20 + i), listened=200)

        data = self._recap(LAST_YEAR).get_json()
        artists = data["tops"]["artists"]
        self.assertEqual(
            [(row["name"], row["listens"], row["tracks"]) for row in artists],
            [("Beta", 5, 1), ("Alpha", 4, 2)],
        )
        albums = data["tops"]["albums"]
        self.assertEqual(
            [(row["title"], row["listens"]) for row in albums],
            [("Beta LP", 5), ("Alpha Second", 3), ("Alpha LP", 1)],
        )
        self.assertEqual(albums[0]["artist"], "Beta")
        self.assertTrue(albums[0]["cover"].startswith("/api/cover/"))


class GenresTestCase(_RecapCase):
    def test_genres_use_measured_styles_and_the_tree(self):
        """``Boogie`` is lifted to ``Disco / funk`` by the Explorer's own
        function; a style the tree does not know is unknown, never displayed."""
        self._analyse(self.tracks["A1"], "Boogie", 0.8)
        self._analyse(self.tracks["A2"], "NotAGenre", 0.9)
        at = datetime.datetime(LAST_YEAR, 9, 1, 8, 0, 0)
        for i in range(10):
            self._event(self.tracks["A1"], at + datetime.timedelta(minutes=i), listened=200)
        for i in range(2):
            self._event(self.tracks["A2"], at + datetime.timedelta(minutes=10 + i), listened=200)
        self._event(self.tracks["A3"], at + datetime.timedelta(minutes=30), listened=200)

        data = self._recap(LAST_YEAR).get_json()
        genres = data["tops"]["genres"]
        # 13 listens, 10 measured: 0.7692 -> the card is offered.
        self.assertTrue(genres["available"])
        self.assertEqual(genres["measured_listens"], 10)
        self.assertEqual(genres["coverage"], round(10 / 13, 4))
        self.assertEqual(genres["explored"], 1)
        self.assertEqual(
            genres["rows"],
            [{"name": "Disco / funk", "family": "disco", "listens": 10, "share": 1.0}],
        )
        self.assertNotIn("Boogie", [row["name"] for row in genres["rows"]])
        self.assertNotIn("NotAGenre", [row["name"] for row in genres["rows"]])
        self.assertIn("genres", data["cards"])

    def test_low_confidence_style_is_unknown(self):
        self._analyse(self.tracks["A1"], "Techno", 0.2)
        at = datetime.datetime(LAST_YEAR, 10, 1, 8, 0, 0)
        for i in range(5):
            self._event(self.tracks["A1"], at + datetime.timedelta(minutes=i), listened=200)
        for i in range(5):
            self._event(self.tracks["A2"], at + datetime.timedelta(minutes=10 + i), listened=200)

        data = self._recap(LAST_YEAR).get_json()
        genres = data["tops"]["genres"]
        self.assertEqual(genres["measured_listens"], 0)
        self.assertEqual(genres["explored"], 0)
        self.assertEqual(genres["rows"], [])
        self.assertFalse(genres["available"])
        self.assertEqual(genres["coverage"], 0.0)
        # 10 x 200 s = 33 min: the year is worth telling, but without a styles card.
        self.assertTrue(data["enough"])
        self.assertNotIn("genres", data["cards"])

    def test_genre_card_absent_below_coverage(self):
        self._analyse(self.tracks["A1"], "Techno", 0.8)
        at = datetime.datetime(LAST_YEAR, 11, 1, 8, 0, 0)
        self._event(self.tracks["A1"], at, listened=200)
        for i in range(9):
            self._event(self.tracks["A2"], at + datetime.timedelta(minutes=10 + i), listened=200)

        data = self._recap(LAST_YEAR).get_json()
        genres = data["tops"]["genres"]
        # A row is computed, but one measured listen out of ten is not a story.
        self.assertEqual(genres["measured_listens"], 1)
        self.assertEqual(genres["coverage"], 0.1)
        self.assertFalse(genres["available"])
        self.assertNotIn("genres", data["cards"])


class PrivacyTestCase(_RecapCase):
    def test_privacy_admin_never_sees_another_person(self):
        at = datetime.datetime(YEAR, 2, 1, 8, 0, 0)
        self._event(self.tracks["A1"], at, listened=200)
        self._event(self.tracks["A1"], at + datetime.timedelta(minutes=1), listened=200)
        for i in range(3):
            self._event(
                self.tracks["B1"],
                at + datetime.timedelta(minutes=10 + i),
                listened=200,
                user=self.bob,
            )

        # Alice is the admin of this server: being an admin gives no view of
        # Bob's year, and `?user=` does not exist.
        mine = self._recap(YEAR)
        data = mine.get_json()
        self.assertEqual(data["totals"]["events"], 2)
        self.assertEqual(
            [row["track"]["title"] for row in data["tops"]["tracks"]], ["A1"]
        )
        self.assertEqual([row["name"] for row in data["tops"]["artists"]], ["Alpha"])
        self.assertEqual(self._recap(YEAR, query="?user=bob").get_json(), data)

        as_bob = self._recap(YEAR, client=self._bob_client()).get_json()
        self.assertEqual(as_bob["totals"]["events"], 3)
        self.assertEqual(
            [row["track"]["title"] for row in as_bob["tops"]["tracks"]], ["B1"]
        )
        self.assertEqual([row["name"] for row in as_bob["tops"]["artists"]], ["Beta"])


class YearsTestCase(_RecapCase):
    def test_years_lists_only_years_with_events_newest_first(self):
        self._event(self.tracks["A1"], datetime.datetime(YEAR, 2, 1, 8, 0, 0), listened=200)
        self._event(
            self.tracks["A1"], datetime.datetime(LAST_YEAR, 2, 1, 8, 0, 0), listened=180
        )
        self._event(
            self.tracks["B1"],
            datetime.datetime(EMPTY_YEAR, 2, 1, 8, 0, 0),
            listened=4,
            outcome=-1,
        )

        data = self._years().get_json()
        self.assertEqual([row["year"] for row in data["years"]], [YEAR, LAST_YEAR, EMPTY_YEAR])
        self.assertEqual(data["latest"], YEAR)
        self.assertEqual(data["now_year"], YEAR)
        self.assertEqual(data["min_year"], 2000)
        self.assertEqual(data["timezone"], "server-local")
        by_year = {row["year"]: row for row in data["years"]}
        self.assertTrue(by_year[YEAR]["partial"])
        self.assertFalse(by_year[LAST_YEAR]["partial"])
        self.assertEqual(by_year[YEAR]["events"], 1)
        self.assertEqual(by_year[YEAR]["listens"], 1)
        self.assertEqual(by_year[YEAR]["minutes"], 3)
        # A year of skips has an event, so it is a chip — with zero listens.
        self.assertEqual(by_year[EMPTY_YEAR]["events"], 1)
        self.assertEqual(by_year[EMPTY_YEAR]["listens"], 0)
        self.assertFalse(by_year[EMPTY_YEAR]["enough"])

    def test_a_row_older_than_the_prune_window_still_gets_a_chip(self):
        """`/years` is NOT floored at `listening.KEEP_DAYS`: a restored server
        (or a prune not yet run) may hold an older row, and the year route can
        serve it, so the chip must exist. 648 days back is past the 548-day
        retention."""
        from neezer import listening

        old_at = NOW - datetime.timedelta(days=listening.KEEP_DAYS + 100)
        self._event(self.tracks["A1"], old_at, listened=200)

        data = self._years().get_json()
        self.assertEqual([row["year"] for row in data["years"]], [old_at.year])
        detail = self._recap(old_at.year).get_json()
        self.assertEqual(detail["totals"]["events"], 1)
        self.assertEqual(detail["first"]["at"], old_at.isoformat())

    def test_a_fresh_account_has_no_year(self):
        as_bob = self._years(client=self._bob_client()).get_json()
        self.assertEqual(as_bob["years"], [])
        self.assertIsNone(as_bob["latest"])
        self.assertEqual(as_bob["now_year"], YEAR)


class ShapeAndCostTestCase(_RecapCase):
    def _full_year(self):
        """20 listens over three consecutive days, all measured, first/last."""
        self._analyse(self.tracks["A1"], "Techno", 0.8)
        self._analyse(self.tracks["A2"], "House", 0.8)
        self._analyse(self.tracks["B1"], "Ambient", 0.8)
        base = datetime.datetime(LAST_YEAR, 3, 1, 8, 0, 0)
        for i in range(20):
            track = (self.tracks["A1"], self.tracks["A2"], self.tracks["B1"])[i % 3]
            self._event(
                track,
                base + datetime.timedelta(days=i % 3, minutes=i),
                listened=200,
            )

    def test_payload_has_the_documented_keys(self):
        self._full_year()
        response = self._recap(LAST_YEAR)
        self.assertEqual(response.status_code, 200)
        data = response.get_json()
        self.assertEqual(
            set(data),
            {
                "year",
                "partial",
                "enough",
                "timezone",
                "period",
                "totals",
                "months",
                "streak",
                "biggest_day",
                "tops",
                "first",
                "last",
                "cards",
                "generated",
            },
        )
        self.assertEqual(
            set(data["totals"]),
            {
                "events",
                "listens",
                "skips",
                "neutrals",
                "seconds",
                "minutes",
                "days_active",
                "first_at",
                "last_at",
            },
        )
        self.assertEqual(set(data["period"]), {"from", "to"})
        self.assertEqual(len(data["months"]), 12)
        self.assertEqual(set(data["months"][0]), {"month", "listens", "skips", "seconds"})
        self.assertEqual(set(data["streak"]), {"days", "from", "to"})
        self.assertEqual(set(data["biggest_day"]), {"date", "listens", "seconds"})
        self.assertEqual(set(data["tops"]), {"tracks", "artists", "albums", "genres"})
        self.assertEqual(set(data["tops"]["tracks"][0]), {"track", "listens", "seconds"})
        self.assertEqual(
            set(data["tops"]["artists"][0]),
            {"id", "name", "cover", "listens", "tracks"},
        )
        self.assertEqual(
            set(data["tops"]["albums"][0]),
            {"id", "title", "artist", "cover", "listens"},
        )
        genres = data["tops"]["genres"]
        self.assertEqual(
            set(genres),
            {"available", "coverage", "measured_listens", "explored", "rows"},
        )
        self.assertEqual(set(genres["rows"][0]), {"name", "family", "listens", "share"})
        self.assertEqual(set(data["first"]), {"at", "track"})
        self.assertEqual(set(data["last"]), {"at", "track"})
        self.assertIsInstance(data["generated"], int)
        # Every card, in the server's fixed order.
        self.assertEqual(data["cards"], CARD_ORDER)
        # The track payload is the standard one the whole API serves.
        track = data["tops"]["tracks"][0]["track"]
        for key in ("id", "deezer_id", "title", "duration", "artist", "album", "quality"):
            self.assertIn(key, track)
        self.assertEqual(track["artist"]["name"], track["display_artist"].split(" feat.")[0])
        self.assertTrue(data["tops"]["albums"][0]["cover"].startswith("/api/cover/"))

    def test_statement_counts(self):
        self._full_year()
        recap.invalidate()
        # The tree is a 30 s process cache: refresh its timestamp OUTSIDE the
        # count so a recheck can never be attributed to the request.
        genretree.get()

        # 1 for the year list, plus the ONE session lookup every /api route
        # pays (login_required reads the user row): 2 on the wire.
        with count_statements() as counts:
            self.assertEqual(self._years().status_code, 200)
        self.assertEqual(dict(counts), {"SELECT": 2}, counts)

        # The recap itself is 4: the daily aggregate, the per-track aggregate,
        # the titles' payloads and their credits. Plus the same session lookup.
        with count_statements() as counts:
            self.assertEqual(self._recap(LAST_YEAR).status_code, 200)
        self.assertEqual(dict(counts), {"SELECT": 5}, counts)

        # The same 4 measured without the session lookup: the aggregation and
        # the serialisation alone, which is the budget the plan states.
        from flask import request as flask_request

        from neezer.webui import recap as recap_routes

        recap.invalidate()
        with self.app.test_request_context():
            flask_request.webuser = self.user
            with count_statements() as counts:
                recap_routes._payload(recap.year_stats(self.user, LAST_YEAR))
        self.assertEqual(dict(counts), {"SELECT": 4}, counts)

        # A second view inside the TTL: the aggregates are cached, the track
        # payloads are NOT (a file archived since must show at once), so only
        # the two payload statements are paid again.
        with count_statements() as counts:
            self.assertEqual(self._recap(LAST_YEAR).status_code, 200)
        self.assertEqual(dict(counts), {"SELECT": 3}, counts)

        # An empty year short-circuits after the daily aggregate: 1 + 1.
        with count_statements() as counts:
            self.assertEqual(self._recap(EMPTY_YEAR).status_code, 200)
        self.assertEqual(dict(counts), {"SELECT": 2}, counts)

        # Nothing here writes, on either route.
        with count_statements() as counts:
            self._years()
            self._recap(LAST_YEAR)
        self.assertEqual(set(counts), {"SELECT"}, counts)

    def test_payload_is_bounded(self):
        """20 000 events must not grow the answer: the tops are capped by
        construction, so the bytes are the five rows, twelve months and two
        titles."""
        base = datetime.datetime(LAST_YEAR, 1, 1, 0, 0, 0)
        tracks = list(self.tracks.values())
        rows = [
            {
                "user": self.user.id,
                "track": tracks[i % len(tracks)].id,
                "at": base + datetime.timedelta(days=i % 300, seconds=i % 86400),
                "listened": 180,
                "outcome": 1 if i % 7 else -1,
            }
            for i in range(20000)
        ]
        with dbmod.db.atomic():
            # SQLite's bound-variable limit: 5 columns x 150 rows stays under it.
            for start in range(0, len(rows), 150):
                ListenEvent.insert_many(rows[start : start + 150]).execute()

        data = self._recap(LAST_YEAR).get_json()
        raw = json.dumps(data)
        self.assertEqual(data["totals"]["events"], 20000)
        self.assertEqual(len(data["months"]), 12)
        for kind in ("tracks", "artists", "albums"):
            self.assertLessEqual(len(data["tops"][kind]), 5, kind)
        # Measured on SQLite: 6 769 bytes for this payload (20 000 events, five
        # titles); the plan's ceiling is 12 000.
        self.assertLessEqual(len(raw), 12000, len(raw))

    def test_second_view_is_an_empty_304(self):
        """`generated` is frozen with the aggregate, so two views inside the
        TTL are byte-identical: the browser's revalidation gets an empty 304,
        measured at 0 bytes."""
        self._full_year()
        recap.invalidate()
        first = self._recap(LAST_YEAR)
        self.assertEqual(first.status_code, 200)
        self.assertEqual(first.headers["Cache-Control"], "private, no-cache")
        etag = first.headers["ETag"]
        self.assertEqual(self._recap(LAST_YEAR).get_json()["generated"], first.get_json()["generated"])

        second = self.client.get(
            f"/api/recap/{LAST_YEAR}", headers={"If-None-Match": etag}
        )
        self.assertEqual(second.status_code, 304)
        self.assertEqual(second.get_data(), b"")
        self.assertEqual(second.headers["ETag"], etag)

        years = self._years()
        years_etag = years.headers["ETag"]
        revalidated = self.client.get(
            "/api/recap/years", headers={"If-None-Match": years_etag}
        )
        self.assertEqual(revalidated.status_code, 304)
        self.assertEqual(revalidated.get_data(), b"")

    def test_sql_has_no_dialect_shortcut(self):
        """``DATE()`` is the one spelling the three engines share: no
        ``strftime``, no ``date_trunc``, no ``DATE_FORMAT``."""
        query = recap.daily_query(
            self.user,
            datetime.datetime(YEAR, 1, 1),
            datetime.datetime(YEAR + 1, 1, 1),
        )
        sql = str(query).lower()
        self.assertIn("date(", sql)
        for dialect in ("strftime", "date_trunc", "date_format"):
            self.assertNotIn(dialect, sql)


class YearChecksTestCase(_RecapCase):
    def test_out_of_range_years_are_400(self):
        for year in (recap.MIN_YEAR - 1, 1999, YEAR + 1):
            response = self._recap(year)
            self.assertEqual(response.status_code, 400, year)
            self.assertEqual(response.get_json(), {"error": "invalid year"})
        # The two bounds themselves are valid.
        self.assertEqual(self._recap(recap.MIN_YEAR).status_code, 200)
        self.assertEqual(self._recap(YEAR).status_code, 200)
        # A non-numeric year is a routing 404, never a 500.
        self.assertEqual(self.client.get("/api/recap/abc").status_code, 404)

    def test_both_routes_need_a_session(self):
        anonymous = self.app.test_client()
        for path in ("/api/recap/years", f"/api/recap/{YEAR}"):
            response = anonymous.get(path)
            self.assertEqual(response.status_code, 401, path)
            self.assertEqual(response.get_json(), {"error": "unauthorized"})


class CacheTestCase(_RecapCase):
    def test_current_year_is_cached_for_a_minute_and_a_finished_one_for_an_hour(self):
        clock = [1000.0]
        with mock.patch("neezer.recap.time.monotonic", side_effect=lambda: clock[0]):
            first = recap.year_stats(self.user, YEAR)
            self.assertIs(recap.year_stats(self.user, YEAR), first)
            clock[0] += 61
            refreshed = recap.year_stats(self.user, YEAR)
            self.assertIsNot(refreshed, first)

            past = recap.year_stats(self.user, LAST_YEAR)
            clock[0] += 3599
            self.assertIs(recap.year_stats(self.user, LAST_YEAR), past)
            clock[0] += 2
            self.assertIsNot(recap.year_stats(self.user, LAST_YEAR), past)

    def test_invalidate_drops_one_person_or_everything(self):
        mine = recap.year_stats(self.user, YEAR)
        years = recap.years(self.user)
        recap.invalidate(self.bob)
        self.assertIs(recap.year_stats(self.user, YEAR), mine)
        self.assertIs(recap.years(self.user), years)
        recap.invalidate(self.user)
        self.assertIsNot(recap.year_stats(self.user, YEAR), mine)
        self.assertIsNot(recap.years(self.user), years)
        recap.invalidate()
        self.assertNotIn((str(self.user.id), YEAR), recap._CACHE)

    def test_the_year_list_lives_a_minute(self):
        clock = [1000.0]
        with mock.patch("neezer.recap.time.monotonic", side_effect=lambda: clock[0]):
            first = recap.years(self.user)
            clock[0] += 59
            self.assertIs(recap.years(self.user), first)
            clock[0] += 2
            self.assertIsNot(recap.years(self.user), first)

    def test_the_cache_holds_32_years_at_most(self):
        """Measured: 40 puts leave 32 entries, the oldest evicted first."""
        recap.invalidate()
        with mock.patch("neezer.recap.time.monotonic", return_value=1000.0):
            for i in range(40):
                recap._cache_put(recap._CACHE, ("someone", i), {}, 60)
        self.assertEqual(len(recap._CACHE), 32)
        self.assertNotIn(("someone", 0), recap._CACHE)
        self.assertIn(("someone", 39), recap._CACHE)
        recap.invalidate()


class SourcesTestCase(unittest.TestCase):
    def test_no_provider_coupling(self):
        """The read of the log must stay importable without the provider
        stack: the detach guard proves no call at runtime, this proves none
        can be reached from the module at all."""
        source = pathlib.Path(recap.__file__).read_text(encoding="utf-8")
        for forbidden in ("deezer.provider", "deezerpy", "requests"):
            self.assertNotIn(forbidden, source)


if __name__ == "__main__":
    unittest.main()
