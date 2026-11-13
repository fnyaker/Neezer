# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic API server.
#
# Distributed under terms of the GNU AGPLv3 license.

"""The background explorer (deezer/explorer.py) and its place in the Flow.

Nothing here touches the network: Deezer's API, the preview download and the
analysis are replaced by recorders, so what is asserted is the rule — which genres
are searched, what is accepted, what is never archived, when it stands down."""

import json
import random
import threading
import time
import unittest
import unittest.mock as mock

from neezer import discover
from neezer.db import Discovery, Meta, Track, TrackAnalysis, now
from neezer.deezer import analysis as ana
from neezer.deezer import explorer as X
from neezer.webui import explorer as W

from .test_discover import _Library, count_statements


class _Gate:
    """A Throttle stand-in: lets ``n`` units through, then says stop."""

    def __init__(self, n=999):
        self.n, self.reason = n, None

    def wait(self):
        self.n -= 1
        return self.n >= 0

    def spent(self, _s):
        pass


def _track(i):
    return {"id": 800000 + i, "title": f"T{i}", "duration": 200,
            "preview": f"https://cdnt-preview.dzcdn.net/api/1/1/{i}.mp3",
            "artist": {"id": 50 + i, "name": f"Art {i}"},
            "album": {"id": 5, "title": "Al", "cover_medium": "u"}}


class _Api:
    def __init__(self, tracks):
        self.tracks = tracks
        self.calls = []

    def search_playlist(self, q, limit=12):
        self.calls.append(("playlists", q))
        return {"data": [{"id": 1, "nb_tracks": 40}]}

    def get_playlist_tracks(self, pid, index=0, limit=-1):
        self.calls.append(("tracks", pid))
        return {"data": self.tracks}

    def search(self, q, limit=25, index=0, **kw):
        self.calls.append(("search", q))
        return {"data": self.tracks}


class _Dz:
    def __init__(self, api):
        self.api = api


class _Provider:
    def __init__(self, tracks):
        self.dz = _Dz(_Api(tracks))

    def available(self):
        return True


class ChoosingTestCase(unittest.TestCase):
    def test_only_genres_the_engine_knows_are_ever_searched(self):
        vocab = [l for l, _a in ana.known_genres()]
        w = X.target_weights({"Frenchcore": 50.0}, {"Frenchcore": 40}, vocab + ["Not a genre"])
        self.assertNotIn("Not a genre", w)
        picks = set()
        rng = random.Random(1)
        for _ in range(60):
            picks.update(X.pick_targets(3, rng, w))
        known = {X.genretree.get().node(v) for v in vocab}
        self.assertTrue(picks <= known)

    def test_it_looks_near_what_is_played_and_where_the_library_is_thin(self):
        vocab = ["Frenchcore", "Uptempo", "Hardstyle", "Jazz / acoustique", "Trance"]
        w = X.target_weights({"Frenchcore": 100.0}, {"Frenchcore": 100}, vocab)
        self.assertGreater(w["Uptempo"], w["Hardstyle"])
        self.assertGreater(w["Hardstyle"], w["Jazz / acoustique"])
        self.assertLess(w["Frenchcore"], w["Uptempo"])  # the well stocked genre is the least worth a look
        flat = X.target_weights({}, {}, vocab)
        self.assertTrue(all(v > 0 for v in flat.values()))

    def test_verification_wants_the_genre_heard_and_confidently(self):
        res = lambda style, conf: {"verdict": {"style": style, "conf": conf}}
        self.assertTrue(X.verify("Frenchcore", res("frenchcore", 0.8))[0])
        self.assertTrue(X.verify("Gabber", res("hardcore", 0.8))[0])         # a direct parent
        self.assertFalse(X.verify("Frenchcore", res("hardstyle", 0.9))[0])    # a cousin
        self.assertFalse(X.verify("Techno", res("house", 0.9))[0])            # a sibling
        self.assertFalse(X.verify("Frenchcore", res("frenchcore", 0.2))[0])   # unsure
        self.assertFalse(X.verify("Frenchcore", res("nonsense", 0.9))[0])

    def test_the_style_levels_tighten_low_and_never_accept_a_cousin(self):
        """Measured tree distances in the shipped tree: child/parent 0.10,
        sibling (Gabber vs Frenchcore) 0.20, cousin (Hardstyle vs Frenchcore)
        0.30. Level 1 only takes the shelf itself; level 5 reaches a sibling;
        the 0.25 ceiling refuses the cousin at every level."""
        res = lambda style, conf: {"verdict": {"style": style, "conf": conf}}
        self.assertTrue(X.verify("Frenchcore", res("frenchcore", 0.8), *X.STYLE_LEVELS[1])[0])
        self.assertFalse(X.verify("Frenchcore", res("gabber", 0.8), *X.STYLE_LEVELS[1])[0])
        self.assertTrue(X.verify("Frenchcore", res("gabber", 0.8), *X.STYLE_LEVELS[5])[0])
        self.assertFalse(X.verify("Frenchcore", res("hardstyle", 0.9), *X.STYLE_LEVELS[5])[0])
        self.assertFalse(X.verify("Frenchcore", res("frenchcore", 0.5), *X.STYLE_LEVELS[1])[0])
        self.assertTrue(X.verify("Frenchcore", res("frenchcore", 0.6), *X.STYLE_LEVELS[3])[0])
        for level in (1, 2, 3, 4, 5):
            dist, conf = X.STYLE_LEVELS[level]
            self.assertLessEqual(dist, 0.25)
            self.assertGreaterEqual(conf, 0.45)
        self.assertEqual([X.STYLE_LEVELS[i][0] for i in range(1, 6)],
                         sorted(X.STYLE_LEVELS[i][0] for i in range(1, 6)))
        for bad in (0, 6, "x", True, None, 2.5):
            with self.assertRaises(ValueError):
                X.style_limits(bad)


class ExploreTestCase(_Library):
    def _run(self, tracks, verdicts, gate=None, archive=None, rng_seed=1, targets=1):
        provider = _Provider(tracks)
        it = iter(verdicts)
        analyze = lambda path: {"verdict": next(it), "bpm": 180.0}
        stats = X.explore_once(
            provider, gate or _Gate(), rng=random.Random(rng_seed), targets=targets,
            per_target=len(tracks), archive=archive, analyze=analyze,
            fetch=lambda url, dest: True, pause=lambda s: None,
        )
        return provider, stats

    def _force_target(self, label):
        import unittest.mock as m

        return m.patch.object(X, "pick_targets", lambda n, rng, weights=None: [label])

    def test_keepers_are_recorded_and_the_rest_never_asked_again(self):
        tracks = [_track(1), _track(2), _track(3)]
        good = {"style": "frenchcore", "conf": 0.8}
        bad = {"style": "jazz", "conf": 0.9}
        with self._force_target("Frenchcore"):
            _api, st = self._run(tracks, [good, bad, good])
            self.assertEqual((st["found"], st["rejected"]), (2, 1))
            rows = {r.deezer_id: r for r in Discovery.select()}
            self.assertEqual(len(rows), 3)
            self.assertEqual(sum(r.state == "new" for r in rows.values()), 2)
            info = json.loads(next(r for r in rows.values() if r.state == "new").payload)
            self.assertTrue(info["title"].startswith("T"))
            self.assertTrue(info["artist"].startswith("Art"))
            _api2, st2 = self._run(tracks, [good] * 3)
            self.assertEqual(st2["found"] + st2["rejected"], 0)  # a second pass fetches nothing new

    def test_nothing_is_archived_unless_the_operator_said_so(self):
        archived = []
        ok = {"style": "frenchcore", "conf": 0.9}
        with self._force_target("Frenchcore"):
            self._run([_track(11), _track(12)], [ok, ok], archive=None)
        self.assertEqual(archived, [])
        with self._force_target("Frenchcore"):
            self._run([_track(21), _track(22)], [ok, {"style": "jazz", "conf": 0.9}], archive=archived.append)
        self.assertEqual(len(archived), 1)  # only the verified one
        self.assertIn(archived[0], ("800021", "800022"))

    def test_it_stands_down_the_moment_the_gate_says_so(self):
        ok = {"style": "frenchcore", "conf": 0.9}
        with self._force_target("Frenchcore"):
            self._run([_track(31), _track(32), _track(33)], [ok, ok, ok], gate=_Gate(3))
        self.assertEqual(Discovery.select().count(), 2)  # the target, then one gate per preview

    def test_a_library_track_is_not_looked_for_again(self):
        t = Track.select().first()
        Track.update(deezer_id="777001").where(Track.id == t.id).execute()
        api = _Api([dict(_track(1), id=777001), _track(2)])
        got = X.find_tracks(api, "Frenchcore", random.Random(1), 5, pause=lambda s: None)
        self.assertEqual([str(g["id"]) for g in got], ["800002"])

    def test_the_flow_offers_finds_like_the_librarys_unplayed_titles(self):
        for i, dz in enumerate(("900001", "900002")):
            Discovery.create(deezer_id=dz, state="new", target="Frenchcore", style="Frenchcore",
                             confidence=0.8, bpm=190.0, found=now(),
                             payload=json.dumps(X.info_of(_track(i + 1))))
        Discovery.create(deezer_id="900003", state="rejected", target="Frenchcore", style="Frenchcore",
                         found=now(), payload=json.dumps(X.info_of(_track(3))))
        discover.invalidate()
        taste = discover.profile(self.user, fresh=True)
        pool = discover.track_candidates(self.user, taste, random.Random(1))
        found = {k: c for k, c in pool.items() if isinstance(k, str) and k.startswith("deezer:9000")}
        self.assertEqual(sorted(found), ["deezer:900001", "deezer:900002"])  # the rejected one never
        c = found["deezer:900001"]
        self.assertEqual((c.provider, c.ref), ("deezer", "900001"))
        self.assertTrue(c.why.startswith("Une découverte"))
        self.assertEqual(c.info["title"], "T1")
        # a guest (content-only) is never offered one
        from neezer.db import User

        guest = User.create(name="guest", password="x", salt="x", admin=False)
        gp = discover.track_candidates(guest, discover.profile(guest, fresh=True), random.Random(1))
        self.assertFalse([k for k in gp if isinstance(k, str) and k.startswith("deezer:9000")])
        # taken into the library (played, liked): no longer a find
        t = Track.select().first()
        Track.update(deezer_id="900001").where(Track.id == t.id).execute()
        discover.invalidate()
        pool = discover.track_candidates(self.user, discover.profile(self.user, fresh=True), random.Random(1))
        self.assertNotIn("deezer:900001", pool)
        X.prune()
        self.assertIsNone(Discovery.get_or_none(Discovery.deezer_id == "900001"))

    def test_the_cached_pool_never_crosses_databases(self):
        import time

        # a copy kept for ANOTHER database (as the previous test's) must not be served
        X._pool_cache = (time.monotonic(), -1, [("999", "Frenchcore", {"title": "stale"})])
        self.assertEqual(X.pool_rows(), [])

    def test_the_explorer_switches_are_the_admins_and_are_cleaned(self):
        r = self.client.put("/api/explorer", json={"enabled": False, "archive": True, "pool": 99999})
        self.assertEqual(r.status_code, 200)
        body = r.json
        self.assertFalse(body["enabled"])
        self.assertTrue(body["archive"])
        self.assertEqual(body["pool"], X.POOL_MAX)
        self.assertTrue(self.client.get("/api/explorer").json["archive"])


class StyleDiscoveryTestCase(_Library):
    """The per-style discovery: two playlists, eight previews, the level's
    ladder. What is asserted is the RULE (which verdict is kept, how many calls,
    what is never a verdict), not the network."""

    class Api:
        def __init__(self, playlists, tracks):
            self.playlists = playlists
            self.tracks = tracks
            self.calls = []
            self.analyses = 0

        def search_playlist(self, q, limit=12):
            self.calls.append(("playlists", q))
            return {"data": self.playlists}

        def get_playlist_tracks(self, pid, index=0, limit=-1):
            self.calls.append(("tracks", pid))
            return {"data": self.tracks.get(str(pid), [])}

    def _playlists(self, n=2, per=6):
        lists = {str(10 + i): [dict(_track(i * 100 + j), id=900000 + i * 100 + j)
                               for j in range(per)] for i in range(n)}
        rows = [{"id": int(pid), "title": f"P{pid}", "nb_tracks": 40,
                 "picture_xl": f"https://x/{pid}.jpg"} for pid in lists]
        return lists, rows

    def _run(self, verdicts, level=5, gate=None, fetch=None):
        lists, rows = self._playlists()
        api = self.Api(rows, lists)
        provider = type("P", (), {"dz": type("D", (), {"api": api})()})()
        it = iter(verdicts)

        def analyze(path):
            api.analyses += 1
            return {"verdict": next(it), "bpm": 190.0}

        got = X.discover_style(
            provider, "Frenchcore", level, gate=gate, rng=random.Random(1),
            analyze=analyze, fetch=fetch or (lambda url, dest: True),
            pause=lambda s: None,
        )
        return got, api

    def test_two_playlists_eight_previews_and_only_the_levels_verdicts(self):
        # Gabber is a sibling (0.20): kept at level 5. Hardstyle is a cousin
        # (0.30): never. Jazz is unrelated.
        verdicts = [{"style": "gabber", "conf": 0.8}] * 3 + \
                   [{"style": "hardstyle", "conf": 0.9}] * 3 + \
                   [{"style": "jazz", "conf": 0.9}] * 2
        got, api = self._run(verdicts, level=5)
        self.assertEqual(len(got), 2)
        self.assertEqual(api.analyses, 8)  # 4 + 4 previews, no more
        self.assertEqual(sum(p["matched"] for p in got), 3)
        self.assertEqual({t["style"] for p in got for t in p["tracks"]}, {"Gabber"})
        for t in (t for p in got for t in p["tracks"]):
            self.assertTrue(t["deezer_id"].isdigit())
            self.assertAlmostEqual(t["confidence"], 0.8)
            self.assertTrue(t["title"].startswith("T"))
        self.assertEqual(sum(1 for c in api.calls if c[0] == "tracks"), 2)
        # the same verdicts at level 1: a sibling is out of reach
        got1, api1 = self._run(verdicts, level=1)
        self.assertEqual(sum(p["matched"] for p in got1), 0)
        self.assertEqual(api1.analyses, 8)

    def test_a_preview_that_cannot_be_fetched_is_not_a_verdict(self):
        calls = []

        def fetch(url, dest):
            calls.append(url)
            return len(calls) != 1  # the first preview never arrives

        verdicts = [{"style": "hardcore", "conf": 0.8}] * 8
        got, api = self._run(verdicts, level=5, fetch=fetch)
        self.assertEqual(sum(p["matched"] for p in got), 7)
        self.assertEqual(len(calls), 8)
        self.assertEqual(api.analyses, 7)  # the missing one is never measured

    def test_the_gate_stops_the_previews_and_nothing_else_is_asked(self):
        got, api = self._run([{"style": "hardcore", "conf": 0.8}] * 8,
                             level=5, gate=_Gate(3))
        self.assertLessEqual(sum(len(p["tracks"]) for p in got), 2)
        self.assertEqual(sum(1 for c in api.calls if c[0] == "tracks"), 1)

    def test_the_known_id_lookup_is_bounded(self):
        verdicts = [{"style": "hardcore", "conf": 0.8}] * 8
        n = count_statements(lambda: self._run(verdicts, level=5))
        # one Track + one Discovery query per playlist that answers, plus the
        # 30 s tree version check (a warm process spends 0 there)
        self.assertLessEqual(n, 6)


class DiscoveryRoutesTestCase(_Library):
    """The HTTP contract: pending then ready, cached in Meta, never a 500 for a
    provider failure, and the background job never holds the request."""

    LEVEL = 2
    STYLE = "Frenchcore"

    def setUp(self):
        super().setUp()
        W._inflight.clear()
        for key in list(Meta.select().where(Meta.key.startswith(W.CACHE_PREFIX))):
            key.delete_instance()

    def _ready(self, style=None, conf=0.8):
        return [{
            "id": "42", "title": "A playlist", "cover": "https://x/c.jpg",
            "nb_tracks": 30, "matched": 1,
            "tracks": [{"deezer_id": "1", "title": "T", "style": style or "Frenchcore",
                        "confidence": conf}],
        }]

    def test_pending_then_ready_and_the_second_poll_asks_nobody(self):
        started, release = threading.Event(), threading.Event()
        calls = []

        def fake(provider, style, level, gate, **_kw):
            calls.append((style, level))
            self.assertEqual(gate.duty, W.STYLE_DUTY)
            self.assertEqual(gate.listen_quiet, W.STYLE_LISTEN_QUIET)
            started.set()
            release.wait(10)
            return self._ready()

        provider = type("P", (), {"available": lambda self: True})()
        with mock.patch.object(X, "discover_style", fake), \
                mock.patch.object(W, "_dz_live", lambda: provider):
            first = self.client.post(
                "/api/explorer/discover", json={"style": self.STYLE, "level": self.LEVEL}
            )
            self.assertEqual(first.status_code, 200)
            self.assertEqual(first.json["state"], "pending")
            self.assertEqual(first.json["playlists"], [])
            self.assertTrue(started.wait(10))
            thread = W._inflight.get((self.STYLE, self.LEVEL))
            self.assertIsNotNone(thread)
            release.set()
            thread.join(10)
            self.assertFalse(thread.is_alive())
            second = self.client.get(
                f"/api/explorer/discover?style={self.STYLE}&level={self.LEVEL}"
            )
        self.assertEqual(second.json["state"], "ready")
        self.assertEqual(second.json["level"], self.LEVEL)
        self.assertEqual(second.json["style"], self.STYLE)
        self.assertEqual(second.json["playlists"], self._ready())
        self.assertNotIn("error", second.json)
        self.assertEqual(calls, [(self.STYLE, self.LEVEL)])  # the cache answered the poll

    def test_the_cache_survives_a_restart_and_expires_after_a_day(self):
        W._cache_write(self.STYLE, self.LEVEL, W._payload(
            self.STYLE, self.LEVEL, self._ready()))
        W._inflight.clear()  # a restart: nothing in memory
        ready = W._cache_read(self.STYLE, self.LEVEL)
        self.assertIsNotNone(ready)
        self.assertEqual(ready["playlists"][0]["id"], "42")
        keys = list(Meta.select(Meta.key, Meta.value)
                    .where(Meta.key.startswith(W._cache_key(self.STYLE, self.LEVEL))))
        self.assertGreater(len(keys), 1)  # split to fit Meta.value
        self.assertTrue(all(len(k.key) <= 32 for k in keys))
        self.assertTrue(all(len(k.value) <= 256 for k in keys))
        pointer = Meta.get(Meta.key == W._cache_key(self.STYLE, self.LEVEL))
        pointer.value = json.dumps({"n": json.loads(pointer.value)["n"],
                                    "at": time.time() - W.CACHE_TTL - 1})
        pointer.save()
        self.assertIsNone(W._cache_read(self.STYLE, self.LEVEL))

    def test_an_unreachable_provider_is_ready_empty_never_a_500(self):
        with mock.patch.object(W, "_dz_live", lambda: None):
            r = self.client.post(
                "/api/explorer/discover", json={"style": self.STYLE, "level": self.LEVEL}
            )
        self.assertEqual(r.status_code, 200)
        self.assertEqual(r.json["state"], "ready")
        self.assertEqual(r.json["playlists"], [])
        self.assertEqual(r.json["error"], "provider")
        self.assertNotIn((self.STYLE, self.LEVEL), W._inflight)

    def test_a_worker_outage_is_cached_for_a_minute_not_a_day(self):
        provider = type("P", (), {"available": lambda self: False})()
        started, release = threading.Event(), threading.Event()

        def down(provider, style, level, gate, **_kw):
            started.set()
            release.wait(10)
            raise RuntimeError("down")

        with mock.patch.object(X, "discover_style", down), \
                mock.patch.object(W, "_dz_live", lambda: provider):
            self.client.post(
                "/api/explorer/discover", json={"style": self.STYLE, "level": self.LEVEL}
            )
            self.assertTrue(started.wait(10))
            thread = W._inflight.get((self.STYLE, self.LEVEL))
            self.assertIsNotNone(thread)
            release.set()
            thread.join(10)
            self.assertFalse(thread.is_alive())
        ready = W._cache_read(self.STYLE, self.LEVEL)
        self.assertEqual(ready["error"], "provider")
        pointer = Meta.get(Meta.key == W._cache_key(self.STYLE, self.LEVEL))
        pointer.value = json.dumps({"n": json.loads(pointer.value)["n"],
                                    "at": time.time() - W.PROVIDER_TTL - 1})
        pointer.save()
        self.assertIsNone(W._cache_read(self.STYLE, self.LEVEL))

    def test_the_worker_runs_the_real_discovery_end_to_end(self):
        class Api:
            def search_playlist(self, q, limit=12):
                return {"data": [{"id": 7, "title": "Core", "nb_tracks": 30,
                                  "picture_big": "https://x/7.jpg"}]}

            def get_playlist_tracks(self, pid, index=0, limit=-1):
                return {"data": [dict(_track(i), id=910000 + i) for i in range(6)]}

        provider = type("P", (), {
            "dz": type("D", (), {"api": Api()})(),
            "available": lambda self: True,
        })()
        verdict = {"verdict": {"style": "frenchcore", "conf": 0.8}, "bpm": 190.0}
        from neezer.deezer import workload

        with mock.patch.object(W, "_dz_live", lambda: provider), \
                mock.patch.object(X, "fetch_preview", lambda p, u, d: True), \
                mock.patch.object(X, "CALL_PAUSE", 0), \
                mock.patch.object(workload, "listening_for", lambda: float("inf")), \
                mock.patch.object(workload, "foreground_busy", lambda: False), \
                mock.patch.object(workload, "load_per_core", lambda: 0.0), \
                mock.patch.object(ana, "analyze_file", lambda path: verdict):
            self.client.post(
                "/api/explorer/discover", json={"style": self.STYLE, "level": self.LEVEL}
            )
            thread = W._inflight.get((self.STYLE, self.LEVEL))
            self.assertIsNotNone(thread)
            thread.join(20)
            self.assertFalse(thread.is_alive())
            ready = self.client.get(
                f"/api/explorer/discover?style={self.STYLE}&level={self.LEVEL}"
            ).json
        self.assertEqual(ready["state"], "ready")
        self.assertEqual(len(ready["playlists"]), 1)
        self.assertEqual(ready["playlists"][0]["matched"], 6)
        self.assertEqual(len(ready["playlists"][0]["tracks"]), 6)
        self.assertEqual(ready["playlists"][0]["tracks"][0]["style"], "Frenchcore")

    def test_at_most_two_discoveries_run_at_once(self):
        release = threading.Event()

        def slow(app, provider, style, level):
            release.wait(10)

        provider = type("P", (), {"available": lambda self: True})()
        try:
            with mock.patch.object(W, "_run", slow), \
                    mock.patch.object(W, "_dz_live", lambda: provider):
                for style in ("Frenchcore", "Techno", "Hardcore"):
                    r = self.client.post(
                        "/api/explorer/discover", json={"style": style, "level": self.LEVEL}
                    )
                    self.assertEqual(r.json["state"], "pending")
                self.assertEqual(len(W._inflight), 2)
                self.assertNotIn(("Hardcore", self.LEVEL), W._inflight)
        finally:
            release.set()
            for thread in list(W._inflight.values()):
                thread.join(5)
            W._inflight.clear()

    def test_junk_is_refused_before_anything_is_scheduled(self):
        with mock.patch.object(W, "_dz_live", lambda: self.fail("must not be asked")):
            for body in ({"style": "not a genre", "level": 2},
                         {"style": "Frenchcore", "level": 0},
                         {"style": "Frenchcore", "level": 6},
                         {"style": "Frenchcore", "level": 2.5},
                         {"style": "Frenchcore", "level": True},
                         {"style": "", "level": 2}, {}):
                self.assertEqual(
                    self.client.post("/api/explorer/discover", json=body).status_code, 400, body
                )
            self.assertEqual(self.client.get("/api/explorer/discover").status_code, 400)
        self.assertEqual(W._inflight, {})

    def test_a_cross_site_post_is_refused(self):
        r = self.client.post(
            "/api/explorer/discover",
            json={"style": self.STYLE, "level": self.LEVEL},
            headers={"Sec-Fetch-Site": "cross-site"},
        )
        self.assertEqual(r.status_code, 403)
        self.assertEqual(W._inflight, {})

    def test_the_screen_is_open_to_any_signed_in_person(self):
        self.assertEqual(self.client.get("/api/explorer/styles").status_code, 200)
        self.client.post("/api/logout")
        self.assertEqual(self.client.get("/api/explorer/styles").status_code, 401)


if __name__ == "__main__":
    unittest.main()
