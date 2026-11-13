# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""The History screen's own read: ``GET /api/history``.

A real log with more than two pages, a second account's log next to it, a
listen/skip/neutral mix and events older than the summary window. Every page
is checked for the property a keyset cursor exists for: no gap and no repeat,
even when a play lands while somebody is paging.
"""

import datetime
import os
import unittest

from neezer import db as dbmod
from neezer.db import Album, Artist, ListenEvent, Track, User
from neezer.managers.user import UserManager

from .test_deezer import count_statements
from .test_detach_a import _AppCase


class _HistoryCase(_AppCase):
    def setUp(self):
        super().setUp()
        from neezer.deezer import library

        self.root = library.get_root_folder(self.archive)
        self.user = User.get(User.name == "alice")
        UserManager.add("bob", "B0bpass", admin=False)
        self.bob = User.get(User.name == "bob")
        self.artists = {}
        self.tracks = {}
        self._make("Alpha", "100", "101", ["A1", "A2", "A3"])
        self._make("Beta", "200", "201", ["B1", "B2"])

    def _make(self, artist_name, artist_ref, album_ref, titles):
        artist = Artist.create(name=artist_name, deezer_id=artist_ref)
        album = Album.create(name=artist_name + " LP", artist=artist, deezer_id=album_ref)
        self.artists[artist_name] = artist
        for i, title in enumerate(titles):
            track = Track.create(
                disc=1,
                number=i + 1,
                title=title,
                duration=210,
                album=album,
                artist=artist,
                bitrate=0,
                path=os.path.join(self.archive, f"{artist_name}-{title}.flac"),
                last_modification=1,
                root_folder=self.root,
                folder=self.root,
            )
            self.tracks[title] = track

    def _event(self, track, when, listened=180, outcome=1, user=None, source="web"):
        return ListenEvent.create(
            user=user or self.user,
            track=track,
            at=when,
            listened=listened,
            outcome=outcome,
            source=source,
        )

    def _history(self, query="", client=None):
        return (client or self.client).get("/api/history" + query)


class PageTestCase(_HistoryCase):
    def test_newest_first_and_the_same_second_is_ordered_by_id(self):
        t = self.tracks["A1"]
        base = dbmod.now() - datetime.timedelta(hours=2)
        first = self._event(t, base - datetime.timedelta(minutes=1))
        same_a = self._event(t, base, listened=190)
        same_b = self._event(t, base, listened=200)
        newest = self._event(t, base + datetime.timedelta(seconds=30))
        data = self._history("?limit=10").get_json()
        self.assertEqual(
            [e["id"] for e in data["events"]],
            [newest.id, same_b.id, same_a.id, first.id],
        )
        event = data["events"][0]
        self.assertEqual(event["at"], newest.at.isoformat())
        self.assertIn("T", event["at"])
        self.assertEqual(event["listened"], 180)
        self.assertEqual(event["outcome"], 1)
        self.assertEqual(event["source"], "web")
        # The standard list payload, not a history-only shape.
        track = event["track"]
        for key in ("id", "deezer_id", "title", "duration", "artist", "album", "quality"):
            self.assertIn(key, track)
        self.assertEqual(track["title"], "A1")
        self.assertEqual(track["artist"]["name"], "Alpha")
        self.assertEqual(track["album"]["title"], "Alpha LP")
        self.assertIsNone(data["next"])

    def test_cursor_pages_without_gap_or_duplicate_while_a_play_lands(self):
        base = dbmod.now() - datetime.timedelta(hours=1)
        expected = []
        for i in range(25):
            title = "A1" if i % 2 else "B2"
            event = self._event(
                self.tracks[title], base - datetime.timedelta(minutes=i),
                listened=180 + i, outcome=1 if i % 3 else -1,
            )
            expected.append(event.id)
        first = self._history("?limit=10").get_json()
        page_ids = [e["id"] for e in first["events"]]
        self.assertEqual(page_ids, expected[:10])
        self.assertIsNotNone(first["next"])
        # A play recorded while somebody is reading page one is NEWER than the
        # cursor. It must not push an event off page two, and it must not make
        # page two start over.
        self._event(self.tracks["A1"], dbmod.now() - datetime.timedelta(minutes=2))
        seen = list(page_ids)
        cursor = first["next"]
        pages = 1
        while cursor:
            data = self._history(
                "?limit=10&before=%s&before_id=%d"
                % (cursor["before"], cursor["before_id"])
            ).get_json()
            seen.extend(e["id"] for e in data["events"])
            cursor = data["next"]
            pages += 1
        self.assertEqual(pages, 3)
        self.assertEqual(seen, expected)
        self.assertEqual(len(set(seen)), 25)

    def test_cursor_is_strict_at_the_boundary(self):
        base = dbmod.now() - datetime.timedelta(hours=1)
        events = [self._event(self.tracks["A1"], base - datetime.timedelta(seconds=i)) for i in range(3)]
        page = self._history("?limit=1").get_json()
        self.assertEqual([e["id"] for e in page["events"]], [events[0].id])
        after = page["next"]
        second = self._history(
            "?limit=1&before=%s&before_id=%d" % (after["before"], after["before_id"])
        ).get_json()
        self.assertEqual([e["id"] for e in second["events"]], [events[1].id])

    def test_cursor_splits_a_second_without_gap(self):
        base = dbmod.now() - datetime.timedelta(hours=1)
        events = [self._event(self.tracks["A1"], base) for _ in range(3)]
        first = self._history("?limit=1").get_json()
        self.assertEqual([e["id"] for e in first["events"]], [events[2].id])
        seen = [events[2].id]
        cursor = first["next"]
        while cursor:
            data = self._history(
                "?limit=1&before=%s&before_id=%d" % (cursor["before"], cursor["before_id"])
            ).get_json()
            seen.extend(e["id"] for e in data["events"])
            cursor = data["next"]
        self.assertEqual(seen, [events[2].id, events[1].id, events[0].id])

    def test_limit_is_capped_and_malformed_arguments_are_refused(self):
        now = dbmod.now()
        for i in range(205):
            self._event(self.tracks["A1"], now - datetime.timedelta(minutes=i))
        default = self._history("").get_json()
        self.assertEqual(len(default["events"]), 50)
        data = self._history("?limit=1000").get_json()
        self.assertEqual(len(data["events"]), 200)
        self.assertIsNotNone(data["next"])
        self.assertEqual(self._history("?limit=abc").status_code, 400)
        self.assertEqual(self._history("?outcome=maybe").status_code, 400)
        self.assertEqual(self._history("?before=2026-11-13T20:31:00").status_code, 400)
        self.assertEqual(self._history("?before_id=12").status_code, 400)
        self.assertEqual(self._history("?before=nope&before_id=12").status_code, 400)


class FilterTestCase(_HistoryCase):
    def test_listens_and_skips_have_their_own_streams(self):
        base = dbmod.now() - datetime.timedelta(hours=1)
        listen = self._event(self.tracks["A1"], base, listened=190, outcome=1)
        skip = self._event(self.tracks["B1"], base - datetime.timedelta(minutes=1), listened=4, outcome=-1)
        neutral = self._event(self.tracks["A2"], base - datetime.timedelta(minutes=2), listened=70, outcome=0)

        every = self._history("?limit=10").get_json()["events"]
        self.assertEqual([e["id"] for e in every], [listen.id, skip.id, neutral.id])
        listens = self._history("?limit=10&outcome=listen").get_json()["events"]
        self.assertEqual([e["id"] for e in listens], [listen.id])
        skips = self._history("?limit=10&outcome=skip").get_json()["events"]
        self.assertEqual([e["id"] for e in skips], [skip.id])

    def test_one_person_never_sees_another_persons_log(self):
        base = dbmod.now() - datetime.timedelta(hours=1)
        mine = self._event(self.tracks["A1"], base)
        theirs = self._event(self.tracks["B1"], base - datetime.timedelta(minutes=1), user=self.bob)

        # Alice is the admin of this server: being an admin gives no view of
        # Bob's history.
        as_alice = self._history("?limit=10").get_json()["events"]
        self.assertEqual([e["id"] for e in as_alice], [mine.id])

        bob_client = self.app.test_client()
        self.assertEqual(
            bob_client.post("/api/login", json={"username": "bob", "password": "B0bpass"}).status_code,
            200,
        )
        as_bob = self._history("?limit=10", client=bob_client).get_json()["events"]
        self.assertEqual([e["id"] for e in as_bob], [theirs.id])

    def test_a_request_without_a_session_is_a_401(self):
        anonymous = self.app.test_client()
        response = anonymous.get("/api/history")
        self.assertEqual(response.status_code, 401)
        self.assertEqual(response.get_json()["error"], "unauthorized")


class SummaryTestCase(_HistoryCase):
    def test_summary_counts_the_window_and_the_top_five(self):
        now = dbmod.now()
        # 12 days inside the window, 40 days outside it.
        self._event(self.tracks["A1"], now - datetime.timedelta(days=1), listened=200, outcome=1)
        self._event(self.tracks["A1"], now - datetime.timedelta(days=2), listened=200, outcome=1)
        self._event(self.tracks["A2"], now - datetime.timedelta(days=3), listened=120, outcome=0)
        self._event(self.tracks["B1"], now - datetime.timedelta(days=4), listened=200, outcome=1)
        self._event(self.tracks["B1"], now - datetime.timedelta(days=5), listened=4, outcome=-1)
        self._event(self.tracks["A3"], now - datetime.timedelta(days=40), listened=999, outcome=1)

        data = self._history("?limit=50").get_json()
        summary = data["summary"]
        self.assertEqual(summary["days"], 30)
        self.assertEqual(summary["listens"], 3)
        self.assertEqual(summary["skips"], 1)
        # 200 + 200 + 120 + 200 + 4 = 724 s -> 12 min; the 40-day event is out.
        self.assertEqual(summary["minutes"], 12)
        self.assertEqual(
            summary["top_artists"],
            [
                {"id": str(self.artists["Alpha"].id), "name": "Alpha",
                 "cover": f"/api/cover/{self.artists['Alpha'].id}", "count": 2},
                {"id": str(self.artists["Beta"].id), "name": "Beta",
                 "cover": f"/api/cover/{self.artists['Beta'].id}", "count": 1},
            ],
        )
        self.assertEqual(
            [(item["track"]["title"], item["count"]) for item in summary["top_tracks"]],
            [("A1", 2), ("B1", 1)],
        )
        self.assertEqual(summary["top_tracks"][0]["track"]["artist"]["name"], "Alpha")

        week = self._history("?limit=50&days=7").get_json()["summary"]
        self.assertEqual(week["days"], 7)
        self.assertEqual(week["listens"], 3)  # days 1, 2, 4 are inside seven days
        self.assertEqual(week["skips"], 1)
        capped = self._history("?limit=50&days=999").get_json()["summary"]
        self.assertEqual(capped["days"], 365)

    def test_top_lists_stop_at_five(self):
        now = dbmod.now()
        for i in range(7):
            artist = Artist.create(name=f"Top{i}", deezer_id=str(700 + i))
            album = Album.create(name=f"Top{i} LP", artist=artist, deezer_id=str(800 + i))
            track = Track.create(
                disc=1, number=1, title=f"Top title {i}", duration=200,
                album=album, artist=artist, bitrate=0,
                path=os.path.join(self.archive, f"top{i}.flac"),
                last_modification=1, root_folder=self.root, folder=self.root,
            )
            self._event(track, now - datetime.timedelta(minutes=i), listened=200, outcome=1)
        summary = self._history("?limit=1").get_json()["summary"]
        self.assertEqual(len(summary["top_artists"]), 5)
        self.assertEqual(len(summary["top_tracks"]), 5)

    def test_no_history_is_an_empty_answer_not_an_error(self):
        data = self._history().get_json()
        self.assertEqual(data["events"], [])
        self.assertIsNone(data["next"])
        self.assertEqual(data["summary"]["listens"], 0)
        self.assertEqual(data["summary"]["skips"], 0)
        self.assertEqual(data["summary"]["minutes"], 0)
        self.assertEqual(data["summary"]["top_artists"], [])
        self.assertEqual(data["summary"]["top_tracks"], [])


class CostTestCase(_HistoryCase):
    def test_a_page_is_a_handful_of_statements_not_one_per_row(self):
        now = dbmod.now()
        for i in range(50):
            self._event(
                self.tracks[("A1", "A2", "A3", "B1", "B2")[i % 5]],
                now - datetime.timedelta(minutes=i), listened=180,
            )
        with count_statements() as counts:
            data = self._history("?limit=50").get_json()
        self.assertEqual(len(data["events"]), 50)
        # Measured: 8 statements for a full page — the page's events, the
        # page's tracks and their credits, the summary aggregate, the artist
        # chart, the title chart and the five titles' payloads and credits.
        # A per-row query would move this to 50+.
        self.assertLessEqual(sum(counts.values()), 10, counts)


if __name__ == "__main__":
    unittest.main()
