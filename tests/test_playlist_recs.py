# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""The bottom of a playlist: titles near the whole playlist, minus its own.

The library is a cluster of seeds (the playlist) and a wider cluster of sound
neighbours, one of them dead (Deezer dropped it) and two of them the seeds
themselves. The section must come back with neighbours only, from the database
alone, and say the same thing twice on the same day.
"""

import datetime
import os
import random
import unittest
from unittest import mock

from neezer import radio
from neezer.db import Album, Artist, Playlist, Track, TrackNeighbor, User, now
from neezer.deezer.provider import DeezerProvider

from .test_detach_a import _AppCase
from .test_detach_guard import Spy
from .test_webui_lists import count_statements

#: 12 artists x 2 titles: enough to fill the 12-row section under
#: mix.assemble's 3-per-artist cap and 3-position gap, and to prove the
#: 20-row ceiling of ?limit=.
CANDIDATES = 24


class PlaylistRecommendationsTestCase(_AppCase):
    def setUp(self):
        super().setUp()
        from neezer.deezer import library

        self.root = library.get_root_folder(self.archive)
        self.user = User.get(User.name == "alice")

        seed_artist = Artist.create(name="Seed")
        seed_album = Album.create(name="Seed LP", artist=seed_artist)
        self.seeds = [
            self.make_track_row(f"Seed {i}", seed_artist, seed_album) for i in range(3)
        ]
        self.candidates = []
        for a in range(CANDIDATES // 2):
            artist = Artist.create(name=f"Near {a}")
            album = Album.create(name=f"Near {a} LP", artist=artist)
            self.candidates += [
                self.make_track_row(f"Near {a}-{i}", artist, album) for i in range(2)
            ]
        # The best neighbour is a dead track: a suggestion must never be one.
        self.unavailable = self.candidates[0]
        self.unavailable.unavailable = now()
        self.unavailable.save(only=(Track.unavailable,))

        # Every seed's stored neighbours: the other seeds (which must be
        # blocked) first, then the whole neighbourhood.
        for seed in self.seeds:
            scored = [(other, 0.99) for other in self.seeds if other != seed]
            scored += [(c, 0.95 - 0.01 * i) for i, c in enumerate(self.candidates)]
            scored.sort(key=lambda pair: -pair[1])
            for rank, (neighbor, score) in enumerate(scored):
                TrackNeighbor.create(track=seed, neighbor=neighbor, score=score, rank=rank)

        self.pl = Playlist.create(user=self.user, name="Mes titres du moment")
        for seed in self.seeds:
            self.pl.add(seed)
        # One title twice: a duplicate occurrence is still one seed (and one
        # exclusion), never a second voice for the same sound.
        self.pl.add(self.seeds[0])

    def make_track_row(self, title, artist, album):
        return Track.create(
            disc=1,
            number=1,
            title=title,
            duration=200,
            has_art=False,
            album=album,
            artist=artist,
            bitrate=0,
            path=os.path.join(self.archive, title + ".flac"),
            last_modification=1,
            root_folder=self.root,
            folder=self.root,
        )

    def url(self, playlist=None, limit=None):
        path = f"/api/playlist/{playlist or self.pl.id}/recommendations"
        return path if limit is None else path + f"?limit={limit}"

    def get(self, playlist=None, limit=None):
        return self.client.get(self.url(playlist, limit)).get_json()

    # -- what it answers -----------------------------------------------------

    def test_it_suggests_neighbours_and_never_the_playlist(self):
        body = self.get()
        self.assertEqual(
            body["playlist"], {"id": str(self.pl.id), "title": self.pl.name}
        )
        ids = [t["id"] for t in body["tracks"]]
        self.assertEqual(len(ids), 12)  # the section's default, filled
        self.assertEqual(len(set(ids)), len(ids))  # no duplicate row
        seeds = {str(s.id) for s in self.seeds}
        self.assertFalse(seeds & set(ids), "a seed came back as a suggestion")
        # The best neighbour is archived but flagged dead: _playable drops it.
        self.assertNotIn(str(self.unavailable.id), ids)

    def test_a_suggestion_is_the_standard_track_payload(self):
        row = self.get()["tracks"][0]
        playlist_row = self.client.get(f"/api/playlist/{self.pl.id}").get_json()["tracks"][0]
        self.assertEqual(set(row), set(playlist_row))
        self.assertTrue(row["title"])
        self.assertTrue(row["artist"]["name"])
        self.assertTrue(row["album"]["title"])
        self.assertIsInstance(row["duration"], int)

    def test_the_limit_is_respected_and_capped_at_twenty(self):
        self.assertEqual(len(self.get(limit=4)["tracks"]), 4)
        capped = self.get(limit=100)["tracks"]
        self.assertEqual(len(capped), 20)
        # Nonsense (and zero) falls back to the default instead of erroring.
        for bad in ("abc", "12.5", "0", "-3", ""):
            self.assertEqual(len(self.get(limit=bad)["tracks"]), 12, bad)

    def test_fewer_than_two_titles_is_an_honest_empty_answer(self):
        one = Playlist.create(user=self.user, name="Un seul")
        one.add(self.seeds[0])
        self.assertEqual(
            self.get(one),
            {"playlist": {"id": str(one.id), "title": "Un seul"}, "tracks": []},
        )
        empty = Playlist.create(user=self.user, name="Vide")
        self.assertEqual(self.get(empty)["tracks"], [])

    def test_an_unknown_or_editorial_playlist_is_a_404(self):
        # Deezer's editorial playlists have no row here, so there is nothing of
        # ours to seed a recommendation from — and nothing to ask Deezer either.
        for pid in (
            "00000000-0000-0000-0000-000000000000",
            "908622995",
            "not-an-id",
        ):
            self.assertEqual(self.client.get(self.url(pid)).status_code, 404, pid)

    def test_a_long_playlist_seeds_a_sample_and_excludes_all_of_it(self):
        # radio.build reading 4 000 seeds measured 2 171 ms on the 8 000-title
        # bench; the sample keeps the section cheap, and the exclusion list
        # still names every row of the playlist.
        from neezer.webui import playlistrecs

        big = Playlist.create(user=self.user, name="Longue")
        rows = list(self.seeds) + list(self.candidates)
        extra_artist = Artist.create(name="More")
        extra_album = Album.create(name="More LP", artist=extra_artist)
        while len(rows) < playlistrecs.MAX_SEEDS + 10:
            rows.append(
                self.make_track_row(f"More {len(rows)}", extra_artist, extra_album)
            )
        for track in rows:
            big.add(track)
        expected = {str(t.id) for t in rows}
        with mock.patch(
            "neezer.webui.playlistrecs.radio.build", return_value=[]
        ) as build:
            body = self.get(big)
        self.assertEqual(body["tracks"], [])
        seeds = build.call_args.args[1]
        self.assertEqual(len(seeds), playlistrecs.MAX_SEEDS)
        self.assertTrue({str(i) for i in seeds} <= expected)
        self.assertEqual({str(i) for i in build.call_args.kwargs["exclude"]}, expected)
        self.assertEqual((seeds[0], seeds[-1]), (rows[0].id, rows[-1].id))

    def test_the_seed_sample_is_deterministic_and_keeps_both_ends(self):
        from neezer.webui import playlistrecs

        ids = list(range(4000))
        sample = playlistrecs._seed_sample(ids)
        self.assertEqual(len(sample), playlistrecs.MAX_SEEDS)
        self.assertEqual(len(set(sample)), len(sample))
        self.assertEqual(sample, sorted(sample))
        self.assertEqual((sample[0], sample[-1]), (0, 3999))
        self.assertEqual(playlistrecs._seed_sample(ids[:3]), [0, 1, 2])

    # -- how it behaves ------------------------------------------------------

    def test_the_database_alone_answers_no_provider(self):
        calls = []
        spy = Spy(calls)
        self.app.deezer._dz = spy
        with mock.patch.object(DeezerProvider, "dz", property(lambda _self: spy)):
            r = self.client.get(self.url())
        self.assertEqual(r.status_code, 200)
        self.assertTrue(r.get_json()["tracks"])
        self.assertEqual(calls, [])

    def test_the_suggestions_are_stable_for_a_fixed_day(self):
        class FixedDate:
            @staticmethod
            def today():
                return datetime.date(2026, 11, 13)

        with mock.patch("neezer.webui.playlistrecs.date", FixedDate):
            first = self.get(limit=6)["tracks"]
            second = self.get(limit=6)["tracks"]
        self.assertEqual([t["id"] for t in first], [t["id"] for t in second])
        # Pinned to the route's own contract: the playlist and the day seed the
        # same draw the radio uses.
        seeds = [s.id for s in self.seeds]
        expected = radio.build(
            self.user,
            seeds,
            n=6,
            exclude=seeds,
            rng=random.Random(f"{self.pl.id}:2026-11-13"),
            block_seeds=True,
        )
        self.assertEqual([t["id"] for t in first], [str(i) for i in expected])

    def test_the_section_costs_a_bounded_number_of_statements(self):
        with count_statements() as counts:
            body = self.get()
        self.assertEqual(len(body["tracks"]), 12)
        # One playlist lookup, the seed list, the radio's own reads (neighbours,
        # two hops, history, dislikes, the candidate rows), then the standard
        # serialisation. Nothing is counted per suggested row.
        self.assertEqual(sum(counts.values()), 12, dict(counts))


if __name__ == "__main__":
    unittest.main()
