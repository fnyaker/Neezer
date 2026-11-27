# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""A track whose loudness was lost gets its gain back.

The failure this pins: an ffmpeg build that aborts inside the ebur128 meter
makes `_spectral` retry WITHOUT it, and that retry legitimately produces every
other descriptor with `lufs: null`. Stored as is, the row is CURRENT — so the
backfill skips it and `regain_from_analyses` has no lufs to read — and
`Track.gain` stays null for good. Both clients then log `gain none` and play
that title unlevelled, which is "la normalisation ne marche plus trop": some
tracks levelled, the ones that lost their meter not.

Measured on a real 440 Hz FLAC through the real ffmpeg: the tone reads
-2.9 LUFS, so its gain is -3.3 dB, and a silent file has no loudness at all
(the repair says so instead of inventing one).
"""

import json
import os
import shutil
import subprocess
import tempfile
import time
import unittest
from unittest import mock

from neezer.db import Track, TrackAnalysis, now
from neezer.deezer import analysis, library

from . import test_webui as _tw

raw_track = _tw.raw_track


def _tone(path, source="sine=frequency=440:duration=3,volume=8", codec=("flac",)):
    """Real audio through the real ffmpeg; returns path."""
    subprocess.run(
        ["ffmpeg", "-v", "error", "-nostdin", "-y", "-f", "lavfi", "-i", source,
         "-c:a", *codec, path],
        check=True,
    )
    return path


class LoudnessRepairTest(unittest.TestCase):
    setUp = _tw.WebUITestCase.setUp
    tearDown = _tw.WebUITestCase.tearDown

    def _needs_ffmpeg(self):
        if not shutil.which("ffmpeg"):
            self.skipTest("ffmpeg")

    def _archived(self, name="tone.flac", source="sine=frequency=440:duration=3,volume=8",
                  audio=True):
        """A Track row on real audio: archived, unanalysed gain, no Deezer.

        ``audio=False`` gives a row that has the shape without paying ffmpeg,
        for the tests that never measure anything (`plan`)."""
        path = os.path.join(self.archive, name)
        if audio:
            _tone(path, source=source)
        with self.app.app_context():
            root = library.get_root_folder(self.archive)
            row = library.upsert_track(raw_track(77, "Up"), root, "FLAC")
            row.path = path
            row.deezer_id = None
            row.gain = None
            row.last_modification = int(time.time())
            row.save()
            return str(row.id)

    def _lost_row(self, track_id, lufs=None):
        """The verdict a dropped meter leaves: current, every measure but lufs."""
        with self.app.app_context():
            row = TrackAnalysis.create(
                track=Track[track_id], version=analysis.ANALYSIS_VERSION,
                analyzed=now(), bpm=0, bpm_confidence=0, bpm_source="x",
                style="x", style_confidence=0, archetype="x",
                data=json.dumps({"centroid": 2000.0, "lufs": lufs, "lra": 7.0}),
            )
            return row

    def test_measure_loudness_on_real_audio_and_silence(self):
        self._needs_ffmpeg()
        path = _tone(os.path.join(self.archive, "tone.flac"))
        lufs, lra = analysis.measure_loudness(path)
        self.assertIsNotNone(lufs)
        # A full-scale sine sits near -3 LUFS; the point is that it is measured
        # from the file, in the real unit, not defaulted.
        self.assertAlmostEqual(lufs, -3.7, delta=1.0)
        self.assertIsNotNone(lra)

        silent = _tone(
            os.path.join(self.archive, "silence.flac"),
            source="anullsrc=r=44100:cl=mono:duration=2",
        )
        lufs, _lra = analysis.measure_loudness(silent)
        # ebur128 floors silence at -70 LUFS; the gain rule refuses that value
        # (`gain_from_lufs`), so a silent file gets no gain rather than a huge one.
        self.assertIsNone(analysis.gain_from_lufs(lufs))

    def test_backfill_repairs_a_current_verdict_that_lost_its_loudness(self):
        self._needs_ffmpeg()
        track_id = self._archived()
        self._lost_row(track_id)

        with self.app.app_context():
            # On master this is the bug: version is current, so every track is
            # skipped, gain stays None, and no client can invent the loudness.
            stats = analysis.backfill(workers=1)
            track = Track[track_id]
            self.assertIsNotNone(track.gain)
            self.assertAlmostEqual(track.gain, -4.1, delta=1.1)
            self.assertEqual(stats["repaired"], 1, stats)
            data = json.loads(TrackAnalysis.get(TrackAnalysis.track == track_id).data)
            self.assertIsNotNone(data["lufs"])
            self.assertAlmostEqual(data["lufs"], -3.7, delta=1.0)

            # Idempotent: the repaired row is current AND has its loudness, so
            # the next pass decodes nothing.
            again = analysis.backfill(workers=1)
            self.assertEqual(again["repaired"], 0, again)
            self.assertEqual(again["skipped"], 1, again)

        # ...and the contract both clients read: the batch gain endpoint now
        # answers a number where it used to answer null.
        self.client.post("/api/login", json={"username": "alice", "password": "Alic3"})
        self.assertAlmostEqual(
            self.client.post("/api/gains", json={"ids": [track_id]}).get_json()["gains"][track_id],
            -4.1, delta=1.1,
        )

    def test_the_plan_counts_a_lost_loudness_so_the_nightly_window_runs(self):
        from neezer.webui import analysis as wa

        track_id = self._archived(audio=False)
        self._lost_row(track_id)
        with self.app.app_context():
            p = wa.plan()
        self.assertEqual(p["lost_loudness"], 1)
        self.assertGreaterEqual(p["todo"], 1)
        self.assertEqual(p["current"], 0)

    def test_spectral_fills_the_loudness_after_the_ebur128_retry(self):
        feats = {
            "centroid": 2000.0, "spread": 1000.0, "flatness": 0.2,
            "flatness_hi": 0.3, "entropy": 2.0, "rolloff": 5000.0,
            "flux_peak": 1.5, "lufs": None, "lra": None, "frames": 300,
        }
        with mock.patch.object(analysis.subprocess, "run") as run, \
                mock.patch.object(analysis, "_parse_spectral",
                                  side_effect=[None, feats]), \
                mock.patch.object(analysis, "measure_loudness",
                                  return_value=(-9.5, 6.0)):
            run.return_value = mock.Mock(returncode=-6, stdout=b"", stderr=b"boom")
            got = analysis._spectral("/archive/t.flac")
        self.assertEqual(got["lufs"], -9.5)
        self.assertEqual(got["lra"], 6.0)
        self.assertEqual(got["centroid"], 2000.0)
