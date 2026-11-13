# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic API server.
#
# Distributed under terms of the GNU AGPLv3 license.

"""The Explorer's parent styles (``GET /api/explorer/styles``).

A leaf is not a style a person lives in: Boogie climbs to Disco/funk, Frenchcore
to Hardcore, and a one-child chain (Rawphase -> Rawstyle) climbs to the family
(Hardstyle). The weights come only from what the person played or starred, the
confidence from the served verdicts, and one account's history never feeds
another's. Measured numbers sit next to the assertions.
"""

import os
import unittest

from neezer import discover, listening
from neezer.db import (
    Album, Artist, StarredTrack, Track, TrackAnalysis,
)
from neezer.managers.user import UserManager

from .test_discover import _Library, count_statements


class _Styles(_Library):
    def setUp(self):
        super().setUp()
        self.bob = UserManager.add("bob", "B0bpass", admin=False)
        self.rows = {}
        self._n = 0

        def add(name, style, n, conf):
            artist = Artist.create(name=name)
            album = Album.create(name=f"{name} LP", artist=artist)
            out = []
            for i in range(n):
                self._n += 1
                t = Track.create(
                    disc=1, number=i + 1, title=f"{name} {i}", duration=200, has_art=False,
                    album=album, artist=artist, bitrate=0,
                    path=os.path.join(self.archive, f"style-{self._n}.flac"),
                    last_modification=1, root_folder=self.root, folder=self.root,
                )
                TrackAnalysis.create(
                    track=t, style=style, style_confidence=conf, version=5
                )
                out.append(t)
            self.rows[name] = out
            return out

        for t in add("Boogie", "Boogie", 4, 0.8):
            listening.record(self.bob, t, 200)
        for t in add("Frenchcore", "Frenchcore", 3, 0.7):
            listening.record(self.bob, t, 200)
        for t in add("Rawphase", "Rawphase", 2, 0.6):
            listening.record(self.bob, t, 200)
        for t in add("Jazz", "jazz", 2, 0.5):
            listening.record(self.bob, t, 200)
        for t in add("Freeform", "Freeform", 1, 0.4):
            listening.record(self.bob, t, 200)
        # A title with no verdict at all: listened, never in a style.
        artist = Artist.create(name="NoVerdict")
        album = Album.create(name="NoVerdict LP", artist=artist)
        t = Track.create(
            disc=1, number=1, title="Not measured", duration=200, has_art=False,
            album=album, artist=artist, bitrate=0,
            path=os.path.join(self.archive, "style-noverdict.flac"),
            last_modification=1, root_folder=self.root, folder=self.root,
        )
        self.rows["NoVerdict"] = [t]
        listening.record(self.bob, t, 200)
        # An explicit star on a title never played: a real signal, not a guess.
        star = add("StarOnlyTechno", "Techno", 1, 0.9)[0]
        StarredTrack.create(user=self.bob, starred=star)
        # A style only alice ever plays: bob must never see it.
        for t in add("AliceOnly", "Rock", 2, 0.8):
            listening.record(self.user, t, 200)
        discover.invalidate()

    def payload(self, user=None):
        return discover.parent_styles(user or self.bob, discover.STYLES_TOP)


class ParentStylesTestCase(_Styles):
    def test_a_leaf_is_served_as_its_parent_and_never_as_itself(self):
        out = self.payload()
        rows = {r["name"]: r for r in out["styles"]}
        self.assertEqual(out["styles"][0]["name"], "Disco / funk")  # Boogie, 4 listens
        self.assertNotIn("Boogie", rows)
        self.assertNotIn("Frenchcore", rows)
        self.assertIn("Hardcore", rows)               # Frenchcore's parent
        self.assertIn("Rawstyle", rows)               # Rawphase's nearest parent — and a family
        self.assertNotIn("Rawphase", rows)
        self.assertIn("Jazz / acoustique", rows)      # the stored family id resolves
        self.assertNotIn("jazz", rows)
        disco = rows["Disco / funk"]
        self.assertEqual((disco["parent"], disco["family"]), ("Funk", "Disco / funk"))
        self.assertEqual((disco["tracks"], disco["children"], disco["leaf"]), (4, 4, False))
        self.assertEqual(rows["Hardcore"]["children"], 12)
        self.assertEqual(rows["Rawstyle"]["children"], 1)  # a family is a shelf even so
        self.assertEqual(disco["confidence"], 0.8)
        self.assertEqual(rows["Hardcore"]["confidence"], 0.7)
        self.assertEqual(rows["Rawstyle"]["confidence"], 0.6)
        # Weights fall, and the top's is 1.0 by construction.
        weights = [r["weight"] for r in out["styles"]]
        self.assertEqual(weights, sorted(weights, reverse=True))
        self.assertEqual(weights[0], 1.0)
        self.assertTrue(all(r["tracks"] > 0 for r in out["styles"]))

    def test_an_unknown_style_is_kept_under_its_own_name_not_mapped(self):
        rows = {r["name"]: r for r in self.payload()["styles"]}
        free = rows["Freeform"]
        self.assertIsNone(free["parent"])
        self.assertIsNone(free["family"])
        self.assertTrue(free["leaf"])
        self.assertEqual(free["children"], 0)
        self.assertEqual(free["tracks"], 1)
        self.assertEqual(free["confidence"], 0.4)

    def test_a_star_weighs_more_than_a_play_and_both_are_the_persons(self):
        out = self.payload()
        rows = {r["name"]: r for r in out["styles"]}
        self.assertIn("Techno", rows)  # the starred title, never played
        # The profile itself: one star adds 2.5 against a played title's 1.0.
        taste = discover.profile(self.bob, fresh=True)
        star = self.rows["StarOnlyTechno"][0].id
        self.assertEqual(taste.tracks[star], taste.tracks[self.rows["Boogie"][0].id] * 2.5)
        self.assertGreater(rows["Techno"]["weight"], rows["Jazz / acoustique"]["weight"])

    def test_the_analysed_share_and_the_unknown_share_are_stated(self):
        out = self.payload()
        # 13 listened titles + the star (2.5): 15.5 of weight; 14.5 analysed
        # (one title has no verdict), 1.0 under a name the tree does not know.
        self.assertAlmostEqual(out["analysed"], 14.5 / 15.5, delta=0.005)
        self.assertAlmostEqual(out["unknown"], 1.0 / 15.5, delta=0.005)
        total = out["tracks"]
        self.assertEqual(total, sum(r["tracks"] for r in out["styles"]))
        self.assertEqual(len(out["scale"]), len(out["styles"]))
        self.assertEqual(out["source"], "history")
        self.assertEqual(out["generated"], int(out["generated"]))

    def test_one_person_one_profile(self):
        bob = {r["name"] for r in self.payload(self.bob)["styles"]}
        alice = {r["name"] for r in self.payload(self.user)["styles"]}
        self.assertNotIn("Rock", bob)          # only alice played it
        self.assertIn("Rock", alice)
        self.assertIn("Disco / funk", bob)     # only bob played it
        self.assertNotIn("Disco / funk", alice)
        self.assertNotIn("Techno", alice)      # bob's star, not hers

    def test_a_person_with_no_history_gets_no_invented_style(self):
        carol = UserManager.add("carol", "C4rolpass", admin=False)
        out = discover.parent_styles(carol, 10)
        self.assertEqual(out["styles"], [])
        self.assertEqual(out["scale"], [])
        self.assertEqual(out["tracks"], 0)
        self.assertEqual(out["analysed"], 0.0)
        self.assertEqual(out["source"], "none")
        # …even though the shared counters are full of alice's plays.
        self.assertGreater(Track.select().where(Track.play_count > 0).count(), 0)

    def test_a_profile_with_history_but_no_verdict_says_unanalysed(self):
        carol = UserManager.add("carol2", "C4rolpass", admin=False)
        for t in self.rows["NoVerdict"]:
            listening.record(carol, t, 200)
        out = discover.parent_styles(carol, 10)
        self.assertEqual(out["styles"], [])
        self.assertEqual(out["analysed"], 0.0)
        self.assertEqual(out["source"], "unanalysed")

    def test_the_read_is_bounded_and_asks_no_provider(self):
        discover.profile(self.bob, fresh=True)  # the profile is what costs
        n = count_statements(lambda: self.payload())
        self.assertLessEqual(n, 2)  # the cached profile, and the 30 s tree check

    def test_the_route_serves_the_payload_and_needs_a_session(self):
        self.client.post("/api/logout")
        self.client.post("/api/login", json={"username": "bob", "password": "B0bpass"})
        r = self.client.get("/api/explorer/styles")
        self.assertEqual(r.status_code, 200)
        body = r.json
        self.assertEqual(set(body), {"styles", "scale", "analysed", "unknown",
                                     "tracks", "source", "generated"})
        self.assertTrue(body["styles"])
        self.assertLessEqual(len(body["styles"]), 10)
        self.assertNotIn("Boogie", {s["name"] for s in body["styles"]})
        again = self.client.get("/api/explorer/styles").json
        self.assertEqual([s["name"] for s in body["styles"]],
                         [s["name"] for s in again["styles"]])
        self.client.post("/api/logout")
        self.assertEqual(self.client.get("/api/explorer/styles").status_code, 401)


if __name__ == "__main__":
    unittest.main()
