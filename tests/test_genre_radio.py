# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Genre stations (``neezer/webui/genre_radio.py``).

A station is STYLE-FIRST: asked for a parent genre it plays that genre and its
sub-genres, never a sibling, a parent or an unrelated style — even though the
Flow's own pool also holds sound neighbours and the library's other genres. It
still follows the person (their discovery level, their skips) and it never
writes the Flow's saved aim. Every answer comes from the database: a station
starts with Deezer off.
"""

import os
import unittest
from urllib.parse import quote

from neezer import discover, genretree
from neezer.db import Album, Artist, Track, TrackAnalysis, UserTaste
from neezer.managers.user import UserManager

from .test_discover import _Library, count_statements


class _Stations(_Library):
    """The shared library plus enough analysed titles to fill a 40-title
    station, so a refill still has something new to give after 40 exclusions."""

    def setUp(self):
        super().setUp()
        artist = Artist.create(name="Station Maker")
        album = Album.create(name="Station Maker LP", artist=artist)
        n = 0
        for style, count in (("Hardcore", 30), ("Frenchcore", 24)):
            for i in range(count):
                n += 1
                t = Track.create(
                    disc=1, number=i + 1, title=f"{style} extra {i}", duration=200,
                    has_art=False, album=album, artist=artist, bitrate=0,
                    path=os.path.join(self.archive, f"station-{n}.flac"),
                    last_modification=1, root_folder=self.root, folder=self.root,
                )
                TrackAnalysis.create(track=t, style=style, version=5)
        discover.invalidate()
        self.by_style = {}
        for tid, style in TrackAnalysis.select(
            TrackAnalysis.track, TrackAnalysis.style
        ).tuples():
            self.by_style.setdefault(style, set()).add(str(tid))

    def branch(self, style):
        """The row ids in ``style`` and everything under it."""
        out = set()
        for name, ids in self.by_style.items():
            if genretree.within(name, style):
                out |= ids
        return out

    def get(self, style, d=None):
        url = "/api/radio/genre/" + quote(style, safe="")
        if d is not None:
            url += "?d=" + quote(str(d))
        return self.client.get(url)

    def next(self, **body):
        return self.client.post("/api/radio/genre/next", json=body)


class StationShapeTestCase(_Stations):
    def test_a_parent_style_keeps_its_children_and_nothing_else(self):
        r = self.get("Hardcore")
        self.assertEqual(r.status_code, 200)
        body = r.json
        self.assertEqual(set(body), {"style", "name", "tracks"})
        self.assertEqual((body["style"], body["name"]), ("Hardcore", "Hardcore"))
        self.assertTrue(body["tracks"])
        self.assertLessEqual(len(body["tracks"]), 40)
        ids = {t["id"] for t in body["tracks"]}
        inside = self.branch("Hardcore")
        self.assertTrue(ids <= inside, ids - inside)
        # The library also holds Techno, Jazz and Classical: none of them may
        # come back, however close the Flow's own pool found them.
        outside = set().union(*(v for k, v in self.by_style.items() if not genretree.within(k, "Hardcore")))
        self.assertFalse(ids & outside)

    def test_a_parent_station_reaches_into_its_subgenre(self):
        """With the parent's own titles and its other child excluded, what is
        left of the branch is Frenchcore: a parent station must still have
        something to play, and it can only be the sub-genre's titles."""
        direct = self.by_style["Hardcore"] | self.by_style["Uptempo"]
        body = self.next(style="Hardcore", exclude=sorted(direct)).json
        ids = {t["id"] for t in body["tracks"]}
        self.assertTrue(ids)
        self.assertTrue(ids <= self.by_style["Frenchcore"], ids - self.by_style["Frenchcore"])

    def test_a_child_style_does_not_climb_to_its_parent_or_siblings(self):
        body = self.get("Frenchcore").json
        ids = {t["id"] for t in body["tracks"]}
        self.assertTrue(ids)
        self.assertTrue(ids <= self.by_style["Frenchcore"])
        self.assertFalse(ids & self.by_style["Hardcore"])
        self.assertFalse(ids & self.by_style["Uptempo"])

    def test_a_station_never_answers_the_same_title_twice(self):
        ids = [t["id"] for t in self.get("Hardcore").json["tracks"]]
        self.assertEqual(len(ids), len(set(ids)))

    def test_a_url_encoded_name_round_trips(self):
        """Spaces and a slash are what a browser sends (``Disco%20%2F%20funk``);
        the node is the tree's own name, never a mangled path."""
        for style in ("Disco / funk", "Électronique"):
            r = self.get(style)
            self.assertEqual(r.status_code, 200)
            self.assertEqual((r.json["style"], r.json["name"]), (style, style))

    def test_an_unknown_style_is_a_404(self):
        r = self.get("Pas un genre")
        self.assertEqual(r.status_code, 404)
        self.assertEqual(r.json, {"error": "unknown style"})
        r = self.next(style="Pas un genre")
        self.assertEqual(r.status_code, 404)
        self.assertEqual(r.json, {"error": "unknown style"})

    def test_an_invalid_level_is_a_400(self):
        self.assertEqual(self.get("Hardcore", d="loin").status_code, 400)
        self.assertEqual(self.next(style="Hardcore", d="loin").status_code, 400)

    def test_a_body_that_is_not_an_object_is_a_400(self):
        r = self.client.post("/api/radio/genre/next", data="Hardcore",
                             content_type="application/json")
        self.assertEqual(r.status_code, 400)
        self.assertEqual(r.json, {"error": "invalid body"})

    def test_a_session_is_required(self):
        self.client.post("/api/logout")
        self.assertEqual(self.get("Hardcore").status_code, 401)
        self.assertEqual(self.next(style="Hardcore").status_code, 401)


class StationRefillTestCase(_Stations):
    def test_next_excludes_what_the_queue_already_holds(self):
        first = [t["id"] for t in self.get("Hardcore").json["tracks"]]
        self.assertTrue(first)
        body = self.next(style="Hardcore", exclude=first).json
        again = [t["id"] for t in body["tracks"]]
        self.assertTrue(again)
        self.assertFalse(set(again) & set(first))
        self.assertTrue(set(again) <= self.branch("Hardcore"))

    def test_next_also_excludes_what_was_just_heard(self):
        heard = [t["id"] for t in self.get("Hardcore").json["tracks"]]
        body = self.next(style="Hardcore", recent=heard).json
        self.assertFalse({t["id"] for t in body["tracks"]} & set(heard))

    def test_next_stays_in_the_branch_and_dedupes(self):
        body = self.next(style="Frenchcore", exclude=[]).json
        ids = [t["id"] for t in body["tracks"]]
        self.assertEqual(len(ids), len(set(ids)))
        self.assertTrue(ids)
        self.assertTrue(set(ids) <= self.by_style["Frenchcore"])

    def test_the_sent_lists_are_capped_not_refused(self):
        """A hostile body cannot grow the work: a 400-id exclude is the cap,
        and anything longer is simply cut (the radio's own rule)."""
        many = [str(t.id) for t in Track.select(Track.id).limit(600)]
        r = self.next(style="Hardcore", exclude=many, recent=many, skipped=many)
        self.assertEqual(r.status_code, 200)
        self.assertLessEqual(len(r.json["tracks"]), 40)


class StationPersonTestCase(_Stations):
    def test_a_person_without_history_still_gets_their_styles_titles(self):
        bob = UserManager.add("bob", "B0bpass", admin=False)
        self.client.post("/api/logout")
        self.client.post("/api/login", json={"username": "bob", "password": "B0bpass"})
        r = self.get("Frenchcore")
        self.assertEqual(r.status_code, 200)
        ids = {t["id"] for t in r.json["tracks"]}
        self.assertTrue(ids)
        self.assertTrue(ids <= self.by_style["Frenchcore"])
        # …and Bob is a person: Alice's history is not his.
        self.assertEqual(discover.profile(bob, fresh=True).tracks, {})

    def test_a_station_never_writes_the_flows_saved_aim(self):
        discover.set_level(self.user, 0.35)
        discover.save_taste(self.user, energy=0.7)
        self.get("Hardcore", d=0.9)
        self.next(style="Hardcore", d=0.9)
        row = UserTaste.get(UserTaste.user == self.user)
        self.assertAlmostEqual(row.discovery, 0.35)
        self.assertAlmostEqual(row.energy, 0.7)

    def test_a_station_never_calls_a_provider_and_works_with_deezer_off(self):
        before = len(self.writes)
        body = self.get("Hardcore").json
        self.assertTrue(body["tracks"])
        self.assertEqual(len(self.writes), before, self.writes.calls[before:])
        self.assertFalse([t for t in body["tracks"] if t["deezer_id"].isdigit()])
        # Deezer switched off entirely: the library still answers.
        self.app.deezer = None
        again = self.get("Frenchcore")
        self.assertEqual(again.status_code, 200)
        self.assertTrue(again.json["tracks"])

    def test_the_read_stays_bounded(self):
        self.get("Hardcore")  # the taste profile is what costs once
        n = count_statements(lambda: self.get("Hardcore"))
        self.assertLessEqual(n, 40)  # measured 29 on this 66-title library


if __name__ == "__main__":
    unittest.main()
