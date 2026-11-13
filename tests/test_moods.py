# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Mood mixes (neezer/moods.py, GET /api/moods and /api/moods/<id>).

The fixture is a library built the way the measures are: each group is ten to
forty records with a known energy, tempo, spectral centroid and flux, so the
test knows by construction which mood may hear which group. Calme and Nuit
share the bottom of the energy scale on purpose (Nuit is its dark side), and
Énergie and Soirée share the high/fast region (Soirée prefers a dance verdict,
it never requires one); every other group belongs to exactly one mood.

What is pinned: the bands, the opening title agreeing with the card's cover,
no duplicate, the artist cap, a day-stable draw, fewer than eight titles is
not a mix, an unknown mood is a 404, and the whole thing costs one statement
for the catalog and none on the second read.
"""

import json
import os
import unittest
from unittest import mock

from neezer import discover, energy, moods
from neezer.db import Album, Artist, Track, TrackAnalysis, User

from .test_deezer import count_statements
from .test_detach_a import _AppCase

DAY = "2026-11-13"


def measures(*, loud, centroid, flux):
    """Analysis data of a record at ``loud`` (0 calm, 1 intense), with its own
    spectral colour and flux — the three axes the moods read."""
    return json.dumps({
        "lufs": -22 + 16 * loud,
        "lra": 15 - 11 * loud,
        "centroid": centroid,
        "fluxPeak": flux,
        "entropy": 0.35 + 0.45 * loud,
    })


class _Library(_AppCase):
    """A library whose every group is built to be heard by one mood."""

    def setUp(self):
        super().setUp()
        from neezer.deezer import library

        self.root = library.get_root_folder(self.archive)
        self.user = User.get(User.name == "alice")
        self.rows = {}
        self.analysis = {}
        self._make_groups()
        energy.invalidate()
        discover.invalidate()
        moods.invalidate()
        self.catalog = moods.catalog(self.user, DAY)

    def _artist(self, name):
        return Artist.create(name=name)

    def _track(self, artist, album, title):
        return Track.create(
            disc=1, number=1, title=title, duration=200, has_art=False,
            album=album, artist=artist, bitrate=0, last_modification=1,
            path=os.path.join(self.archive, title.replace(" ", "_") + ".flac"),
            root_folder=self.root, folder=self.root,
        )

    def _add(self, group, index, *, loud, bpm, centroid, flux, style=None,
             artist=None):
        artist = artist or self._artist(f"{group} artist {index}")
        album = Album.create(name=f"{group} {index} LP", artist=artist)
        track = self._track(artist, album, f"{group} {index}")
        row = TrackAnalysis.create(
            track=track, bpm=bpm, bpm_confidence=0.95, style=style, version=5,
            data=measures(loud=loud, centroid=centroid, flux=flux),
        )
        self.rows.setdefault(group, []).append(track)
        self.analysis[track.id] = row
        return track

    def _make_groups(self):
        # 12 calm records, bright; 12 night records, dark: both at the bottom
        # of the energy scale, which is the two moods' shared ground.
        for i in range(12):
            self._add("calme", i, loud=0.00 + 0.003 * i, bpm=72 + i, centroid=2800,
                      flux=0.8)
        for i in range(12):
            self._add("nuit", i, loud=0.02 + 0.005 * i, bpm=60 + i, centroid=380,
                      flux=1.0)
        # Moderate, steady, not bright: Concentration's own corner.
        for i in range(10):
            self._add("concentration", i, loud=0.78 + 0.005 * i, bpm=100 + i,
                      centroid=2200, flux=1.2)
        # Bright and lively, but not the loudest quarter.
        for i in range(10):
            self._add("bonne-humeur", i, loud=0.76 + 0.005 * i, bpm=112,
                      centroid=4400, flux=1.6)
        # Two high-energy groups: Énergie's songs (one prolific artist, so the
        # cap has something to cap) and Soirée's dance verdicts.
        prolific = self._artist("E0 prolific")
        self.prolific = prolific
        for i in range(8):
            self._add("energie", i, loud=0.85 + 0.002 * i, bpm=126 + i,
                      centroid=2500, flux=2.6, style="Rock", artist=prolific)
        for i in range(32):
            self._add("energie", 8 + i, loud=0.80 + 0.0015 * i, bpm=124 + (i % 12),
                      centroid=2500, flux=2.6, style="Rock")
        for i in range(10):
            self._add("soiree", i, loud=0.85 + 0.002 * i, bpm=128 + i,
                      centroid=3000, flux=3.0,
                      style="House" if i % 2 == 0 else "Techno")

    def group_of(self, track_id):
        for group, rows in self.rows.items():
            if any(t.id == track_id for t in rows):
                return group
        return None


class BandsTestCase(_Library):
    def test_each_mood_only_takes_its_own_band(self):
        # Énergie and Soirée share the high/fast region by design; Nuit is the
        # dark side of Calme's energy floor. Every other group must be absent.
        allowed = {
            "calme": {"calme"},
            "concentration": {"concentration"},
            "bonne-humeur": {"bonne-humeur"},
            "energie": {"energie", "soiree"},
            "soiree": {"soiree", "energie"},
            "nuit": {"nuit"},
        }
        for mood in moods.MOODS:
            ids = self.catalog.mix_ids(mood.id, 40)
            got = {self.group_of(tid) for tid in ids}
            self.assertTrue(ids, mood.id)
            self.assertLessEqual(got, allowed[mood.id], (mood.id, got))

    def test_a_group_is_heard_by_the_mood_it_was_built_for(self):
        for mood_id in ("concentration", "bonne-humeur", "nuit"):
            ids = set(self.catalog.mix_ids(mood_id, 40))
            expected = {t.id for t in self.rows[mood_id]}
            self.assertEqual(ids, expected, mood_id)
        # Calme has no night record (they are dark) and all its own.
        calm = set(self.catalog.mix_ids("calme", 40))
        self.assertEqual(calm, {t.id for t in self.rows["calme"]})
        # Soirée/Énergie draw the same high/fast pool, and the dance verdict is
        # a preference, never a requirement: both groups can appear.
        high = set(self.catalog.mix_ids("energie", 40))
        self.assertTrue(high)
        self.assertTrue(high <= {t.id for t in self.rows["energie"] + self.rows["soiree"]})

    def test_dance_verdict_is_a_preference(self):
        self.assertTrue(moods._is_dance("House"))
        self.assertTrue(moods._is_dance("Techno"))
        self.assertTrue(moods._is_dance("Drum & bass"))
        self.assertFalse(moods._is_dance("Rock"))
        self.assertFalse(moods._is_dance(None))
        # Same title and bands, only the verdict differs: the dance one weighs
        # more, so Soirée opens on it more often than not.
        base = moods._Features(energy=0.9, bpm=130, centroid=0.5, flux=0.8,
                               style="Rock")
        dance = moods._Features(energy=0.9, bpm=130, centroid=0.5, flux=0.8,
                                style="House")
        self.assertGreater(
            moods._score(dance, moods.definition("soiree"), None, None),
            moods._score(base, moods.definition("soiree"), None, None),
        )

    def test_a_title_without_its_measure_never_enters(self):
        # A title with a missing centroid cannot be "Nuit" (darkness unmeasured)
        # or "Bonne humeur" (brightness unmeasured); without a trusted tempo it
        # cannot be "Concentration" or "Énergie".
        no_centroid = moods._Features(energy=0.1, bpm=70, centroid=None, flux=0.4,
                                      style=None)
        no_tempo = moods._Features(energy=0.45, bpm=None, centroid=0.4, flux=0.3,
                                   style=None)
        no_flux = moods._Features(energy=0.1, bpm=70, centroid=0.5, flux=None,
                                  style=None)
        self.assertFalse(moods.definition("nuit").fits(no_centroid))
        self.assertFalse(moods.definition("bonne-humeur").fits(no_centroid))
        self.assertFalse(moods.definition("concentration").fits(no_tempo))
        self.assertFalse(moods.definition("energie").fits(no_tempo))
        # Flux is Concentration's measure, not Calme's: a calm record without
        # one is still calm, a title missing the measure a mood is ABOUT is not.
        self.assertTrue(moods.definition("calme").fits(no_flux))


class AssemblyTestCase(_Library):
    def test_no_duplicate_and_the_artist_cap_holds(self):
        ids = self.catalog.mix_ids("energie", 20)
        self.assertEqual(len(ids), len(set(ids)))
        # E0 has eight matching titles; the cap keeps it at three when the
        # high/fast pool has enough other artists (it does: 28 titles).
        by_artist = {}
        for tid in ids:
            track = next(t for rows in self.rows.values() for t in rows if t.id == tid)
            by_artist[track.artist_id] = by_artist.get(track.artist_id, 0) + 1
        self.assertLessEqual(max(by_artist.values()), 3, by_artist)
        # The prolific artist has eight matching titles: it is exactly capped,
        # not merely rare because the mix is one title per artist.
        self.assertEqual(by_artist.get(self.prolific.id, 0), 3, by_artist)

    def test_no_artist_comes_back_within_three_places(self):
        ids = self.catalog.mix_ids("energie", 20)
        tracks = {t.id: t for rows in self.rows.values() for t in rows}
        last = {}
        for i, tid in enumerate(ids):
            artist = tracks[tid].artist_id
            if artist in last:
                self.assertGreaterEqual(i - last[artist], 3, (artist, last[artist], i))
            last[artist] = i

    def test_the_card_cover_is_the_first_title_of_the_page(self):
        for mood in moods.MOODS:
            entry = self.catalog.get(mood.id)
            self.assertIsNotNone(entry.opening, mood.id)
            self.assertEqual(
                self.catalog.mix_ids(mood.id, 40)[0],
                entry.opening.track,
                mood.id,
            )

    def test_the_draw_is_stable_for_a_day_and_new_the_next(self):
        first = self.catalog.mix_ids("energie", 20)
        moods.invalidate(self.user)
        again = moods.catalog(self.user, DAY)
        self.assertEqual(first, again.mix_ids("energie", 20))
        tomorrow = moods.catalog(self.user, "2026-11-14")
        self.assertNotEqual(first, tomorrow.mix_ids("energie", 20))

    def test_fewer_than_eight_titles_is_not_a_mix(self):
        # Eight is still a mix, seven is not: a mood that thin is not offered
        # rather than played as a short list.
        for track in self.rows["calme"][8:]:
            TrackAnalysis.delete().where(TrackAnalysis.track == track.id).execute()
        moods.invalidate(self.user)
        catalog = moods.catalog(self.user, DAY)
        self.assertEqual(catalog.get("calme").tracks, 8)
        self.assertTrue(catalog.get("calme").ready)
        TrackAnalysis.delete().where(
            TrackAnalysis.track == self.rows["calme"][7].id
        ).execute()
        moods.invalidate(self.user)
        catalog = moods.catalog(self.user, DAY)
        self.assertEqual(catalog.get("calme").tracks, 7)
        self.assertFalse(catalog.get("calme").ready)
        self.assertEqual(catalog.mix_ids("calme", 40), [])

        # And the detail route says why, rather than playing a short mix.
        rv = self.client.get("/api/moods/calme")
        self.assertEqual(rv.status_code, 200)
        self.assertEqual(rv.get_json()["tracks"], [])
        self.assertEqual(rv.get_json()["reason"], "not-enough")


class RoutesTestCase(_Library):
    def test_catalog_payload_and_404(self):
        rv = self.client.get("/api/moods")
        self.assertEqual(rv.status_code, 200)
        body = rv.get_json()["moods"]
        self.assertEqual([m["id"] for m in body],
                         ["calme", "concentration", "bonne-humeur", "energie",
                          "soiree", "nuit"])
        by_id = {m["id"]: m for m in body}
        self.assertTrue(by_id["energie"]["ready"])
        self.assertGreaterEqual(by_id["energie"]["tracks"], 8)
        self.assertTrue(by_id["calme"]["cover"].startswith("/api/cover/"))
        self.assertEqual(self.client.get("/api/moods/unknown").status_code, 404)
        self.assertEqual(self.client.get("/api/moods/energie?limit=nope").status_code, 400)

    def test_detail_payload_is_a_standard_track_list(self):
        rv = self.client.get("/api/moods/energie?limit=12")
        self.assertEqual(rv.status_code, 200)
        body = rv.get_json()
        self.assertEqual(body["id"], "energie")
        self.assertTrue(body["ready"])
        self.assertLessEqual(len(body["tracks"]), 12)
        first = body["tracks"][0]
        # The standard list shape the player consumes: an id, a title, an
        # artist and an album with a same-origin cover.
        for key in ("id", "title", "duration", "artist", "album"):
            self.assertIn(key, first)
        self.assertTrue(first["album"]["cover"].startswith("/api/cover/"))

    def test_the_card_cover_is_the_page_first_cover(self):
        cards = self.client.get("/api/moods").get_json()["moods"]
        checked = 0
        for card in cards:
            if not card["ready"]:
                continue
            detail = self.client.get(f"/api/moods/{card['id']}?limit=1").get_json()
            self.assertEqual(len(detail["tracks"]), 1, card["id"])
            self.assertEqual(card["cover"], detail["tracks"][0]["album"]["cover"],
                             card["id"])
            checked += 1
        self.assertEqual(checked, 6)

    def test_the_route_never_asks_a_provider(self):
        from neezer import webui

        with mock.patch.object(webui, "_provider",
                               side_effect=AssertionError("provider asked")):
            self.assertEqual(self.client.get("/api/moods").status_code, 200)
            self.assertEqual(self.client.get("/api/moods/calme").status_code, 200)
        self.assertEqual(self.writes.calls, [])

    def test_a_private_upload_stays_out_of_another_persons_moods(self):
        from neezer.managers.user import UserManager

        artist = self._artist("Alice only")
        album = Album.create(name="Private LP", artist=artist)
        private = self._track(artist, album, "private calm")
        private.owner = self.user
        private.save()
        TrackAnalysis.create(
            track=private, bpm=75, bpm_confidence=0.95, version=5,
            data=measures(loud=0.01, centroid=2800, flux=0.8),
        )
        energy.invalidate()
        moods.invalidate(self.user)
        alice = self.client.get("/api/moods/calme?limit=40").get_json()["tracks"]
        self.assertIn(str(private.id), {t["id"] for t in alice})

        UserManager.add("bob", "B0b", admin=False)
        bob = self.app.test_client()
        self.assertEqual(
            bob.post("/api/login", json={"username": "bob", "password": "B0b"}).status_code,
            200,
        )
        theirs = bob.get("/api/moods/calme?limit=40").get_json()["tracks"]
        self.assertNotIn(str(private.id), {t["id"] for t in theirs})

    def test_the_catalog_is_one_statement_then_none(self):
        with self.app.app_context():
            discover.profile(self.user)
            energy.energies()
            moods.invalidate(self.user)
            with count_statements() as first:
                moods.catalog(self.user, DAY)
            with count_statements() as second:
                moods.catalog(self.user, DAY)
        self.assertEqual(sum(first.values()), 1, first)
        self.assertEqual(sum(second.values()), 0, second)

    def test_the_warm_routes_never_scan_the_library_again(self):
        # The catalog is built by setUp. A home visit is the login's user read
        # and nothing else; a mood page is that plus the sound-neighbour pairs
        # of its 40 titles, the 40 rows and their credits — never a query per
        # row and never a second pass over track_analysis.
        with count_statements() as cards:
            self.assertEqual(self.client.get("/api/moods").status_code, 200)
        self.assertEqual(sum(cards.values()), 1, cards)
        with count_statements() as detail:
            rv = self.client.get("/api/moods/energie?limit=40")
        self.assertEqual(sum(detail.values()), 4, detail)
        ids = self.catalog.mix_ids("energie", 40)
        self.assertEqual([t["id"] for t in rv.get_json()["tracks"]], [str(i) for i in ids])


class RefusesToFakeTestCase(unittest.TestCase):
    def test_the_catalogue_makes_no_instrumental_or_key_claim(self):
        for mood in moods.MOODS:
            words = (mood.name + " " + mood.subtitle).lower()
            for forbidden in ("instrumental", "sans paroles", "majeur", "mineur",
                              "triste", "joyeux"):
                self.assertNotIn(forbidden, words, mood.id)


if __name__ == "__main__":
    unittest.main()
