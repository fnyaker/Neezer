# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the AGPLv3 license.

"""Seeking in a track, on the server side: byte ranges, `?start=`, `/ready`, and a
transcode that fails.

Every test here runs on a REAL FLAC written by ffmpeg with a marker in it — digital
silence for the first 8 s, then a 1 kHz tone for 4 s — so "the seek landed where it
was asked" is a statement about the sound: a slice taken at 7 s still has 1.000 s of
silence in front of the tone, a slice at 9 s has none, and the durations add up.
"""

import os
import shutil
import subprocess
import tempfile
import unittest
from unittest import mock

from neezer.db import Track

from .test_detach_a import _AppCase

FFMPEG = shutil.which("ffmpeg")
FFPROBE = shutil.which("ffprobe")


def _need_ffmpeg():
    if FFMPEG and FFPROBE:
        return
    if os.environ.get("NS_REQUIRE_FFMPEG") == "1":
        raise RuntimeError("NS_REQUIRE_FFMPEG=1 but ffmpeg/ffprobe are not on PATH")
    raise unittest.SkipTest("ffmpeg/ffprobe not installed")


def _write_marked_flac(path, silence=8, tone=4):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    subprocess.run(
        [
            FFMPEG, "-v", "0", "-f", "lavfi",
            "-i", f"sine=frequency=1000:duration={tone}",
            "-af", f"adelay={int(silence * 1000)}|{int(silence * 1000)}",
            "-c:a", "flac", path, "-y",
        ],
        check=True,
    )


def _decoded(data):
    """``(seconds of audio, seconds of leading silence or None)`` actually decoded.

    A FLAC written to a pipe has no total-samples field (ffmpeg cannot seek back
    to patch STREAMINFO), so the duration has to come from decoding the body —
    which is also the proof that the sound itself is the window asked for.
    """
    with tempfile.NamedTemporaryFile(suffix=".bin", delete=False) as fp:
        fp.write(data)
        path = fp.name
    try:
        out = subprocess.run(
            [FFMPEG, "-v", "info", "-i", path,
             "-af", "silencedetect=noise=-40dB:d=0.05", "-f", "null", "-"],
            capture_output=True, text=True,
        ).stderr
    finally:
        os.remove(path)
    total = 0.0
    for line in out.replace("\r", "\n").splitlines():
        marker = line.find("time=")
        if marker < 0:
            continue
        stamp = line[marker + 5 :].split(" ")[0]
        try:
            hours, minutes, seconds = stamp.split(":")
            total = max(total, int(hours) * 3600 + int(minutes) * 60 + float(seconds))
        except ValueError:
            continue
    ends = [line for line in out.splitlines() if "silence_end:" in line]
    leading = None if not ends else float(ends[0].split("silence_end:")[1].split("|")[0])
    return total, leading


class _SeekCase(_AppCase):
    def setUp(self):
        _need_ffmpeg()
        super().setUp()

        class _Prefetch:
            def download_ids(self, ids, priority=None):
                return len(list(ids))

            def download_episode_ids(self, ids, priority=None):
                return len(list(ids))

            def enqueue_many(self, tracks, count):
                return 0

        self.app.deezer_prefetch = _Prefetch()
        self.track = self.make_track("4242", "Marked")
        self.track.duration = 12
        self.track.bitrate = 1411
        self.track.save(only=(Track.duration, Track.bitrate))
        _write_marked_flac(self.track.path)
        self.size = os.path.getsize(self.track.path)

    def _get(self, query="", **kwargs):
        return self.client.get(f"/api/stream/4242{query}", **kwargs)

    def _body(self, rv):
        if hasattr(rv, "get_data"):
            return rv.get_data()
        return rv.data


class ArchiveRangeTestCase(_SeekCase):
    def test_an_archived_flac_serves_byte_ranges_byte_for_byte(self):
        with open(self.track.path, "rb") as fh:
            whole = fh.read()

        rv = self._get()
        self.assertEqual(rv.status_code, 200)
        self.assertEqual(rv.headers["Accept-Ranges"], "bytes")
        self.assertEqual(int(rv.headers["Content-Length"]), self.size)
        self.assertEqual(self._body(rv), whole)

        rv = self._get(headers={"Range": "bytes=1000-2999"})
        self.assertEqual(rv.status_code, 206)
        self.assertEqual(
            rv.headers["Content-Range"], f"bytes 1000-2999/{self.size}"
        )
        body = self._body(rv)
        self.assertEqual(len(body), 2000)
        self.assertEqual(body, whole[1000:3000])

        rv = self._get(headers={"Range": f"bytes={self.size - 500}-"})
        self.assertEqual(rv.status_code, 206)
        self.assertEqual(self._body(rv), whole[-500:])

    def test_a_range_past_the_end_says_the_real_length(self):
        rv = self._get(headers={"Range": f"bytes={self.size + 500}-"})
        self.assertEqual(rv.status_code, 416)
        self.assertEqual(rv.headers["Content-Range"], f"bytes */{self.size}")

    def test_if_range_keeps_its_promise(self):
        etag = self._get().headers["ETag"]
        rv = self._get(headers={"Range": "bytes=100-199", "If-Range": etag})
        self.assertEqual(rv.status_code, 206)
        self.assertEqual(len(self._body(rv)), 100)

        rv = self._get(headers={"Range": "bytes=100-199", "If-Range": '"other"'})
        self.assertEqual(rv.status_code, 200)
        self.assertEqual(len(self._body(rv)), self.size)

    def test_a_seek_mid_track_costs_only_the_bytes_asked_for(self):
        rv = self._get(headers={"Range": "bytes=9000-"})
        self.assertEqual(rv.status_code, 206)
        body = self._body(rv)
        self.assertEqual(len(body), self.size - 9000)
        self.assertLess(len(body), self.size)


class StartOffsetTestCase(_SeekCase):
    def test_start_lands_on_the_requested_sample_of_a_flac(self):
        rv = self._get("?start=7")
        self.assertEqual(rv.status_code, 200)
        self.assertEqual(rv.mimetype, "audio/flac")
        self.assertEqual(rv.headers["X-NS-Start"], "7.000")
        self.assertEqual(rv.headers["Accept-Ranges"], "none")
        self.assertNotIn("Content-Length", rv.headers)
        body = self._body(rv)
        self.assertTrue(body.startswith(b"fLaC"))
        duration, leading = _decoded(body)
        self.assertAlmostEqual(duration, 5.0, delta=0.05)
        self.assertAlmostEqual(leading, 1.0, delta=0.02)

    def test_start_past_the_marker_opens_on_sound(self):
        body = self._body(self._get("?start=9"))
        duration, leading = _decoded(body)
        self.assertAlmostEqual(duration, 3.0, delta=0.05)
        self.assertIsNone(leading)

    def test_start_is_clamped_to_the_tracks_own_length(self):
        rv = self._get("?start=99")
        self.assertEqual(rv.status_code, 200)
        self.assertEqual(rv.headers["X-NS-Start"], "11.750")

    def test_a_junk_start_is_refused(self):
        for bad in ("abc", "-1", "nan", "inf", ""):
            rv = self._get("?start=" + bad)
            if bad == "":
                self.assertEqual(rv.status_code, 200)
            else:
                self.assertEqual(rv.status_code, 400, bad)
                self.assertIn("start", rv.get_json()["error"])

    def test_an_opus_seek_starts_where_asked_and_never_becomes_the_cache(self):
        from neezer.deezer import ids

        rv = self._get("?q=OPUS_128&start=9")
        self.assertEqual(rv.status_code, 200)
        self.assertEqual(rv.mimetype, "audio/ogg")
        self.assertEqual(rv.headers["X-NS-Start"], "9.000")
        self.assertEqual(rv.headers["Accept-Ranges"], "none")
        self.assertEqual(rv.headers["Cache-Control"], "no-store")
        body = self._body(rv)
        self.assertTrue(body.startswith(b"OggS"))
        duration, leading = _decoded(body)
        self.assertAlmostEqual(duration, 3.0, delta=0.1)
        self.assertIsNone(leading)
        key = f"deezer-{ids.track_uuid('4242')}-opus128.ogg"
        self.assertFalse(
            self.app.transcode_cache.has(key), "a slice must never be the cached track"
        )

    def test_a_cached_transcode_does_not_answer_a_seek(self):
        self._body(self._get("?q=OPUS_128"))
        from neezer.deezer import ids

        key = f"deezer-{ids.track_uuid('4242')}-opus128.ogg"
        self.assertTrue(self.app.transcode_cache.has(key), "the whole track is cached")

        rv = self._get("?q=OPUS_128&start=9")
        self.assertEqual(rv.status_code, 200)
        self.assertEqual(rv.headers["X-NS-Start"], "9.000")
        duration, _ = _decoded(self._body(rv))
        self.assertAlmostEqual(
            duration, 3.0, delta=0.1, msg="the cached whole track was served, not a slice"
        )

    def test_a_seek_while_the_archive_is_still_coming_is_a_clear_refusal(self):
        os.remove(self.track.path)
        from neezer.deezer import archive

        def live(provider, track, on_abort=None):  # pragma: no cover - must not run
            raise AssertionError("a seek must not start the live stream again")

        with mock.patch.object(archive, "open_live_stream", live):
            rv = self._get("?start=5")
        self.assertEqual(rv.status_code, 409)
        self.assertEqual(rv.headers["Retry-After"], "2")
        payload = rv.get_json()
        self.assertFalse(payload["ready"])
        self.assertFalse(
            payload["streaming"], "nothing is downloading this track yet"
        )

        with open(f"{self.track.path}.deadbeef.part", "wb") as fh:
            fh.write(b"\0" * 10)
        with mock.patch.object(archive, "open_live_stream", live):
            payload = self._get("?start=5").get_json()
        self.assertTrue(
            payload["streaming"], "an archive is being written: the wait ends"
        )


class GeneratedStreamTestCase(_SeekCase):
    def _live(self, payload):
        from neezer.deezer import archive

        def live(provider, track, on_abort=None):
            def gen():
                for i in range(0, len(payload), 2048):
                    yield payload[i : i + 2048]

            return "audio/flac", gen()

        return mock.patch.object(archive, "open_live_stream", live)

    def test_a_generated_stream_declares_that_it_has_no_ranges(self):
        with open(self.track.path, "rb") as fh:
            payload = fh.read()
        os.remove(self.track.path)
        with self._live(payload):
            rv = self._get()
        self.assertEqual(rv.status_code, 200)
        self.assertEqual(rv.headers["Accept-Ranges"], "none")
        self.assertEqual(rv.headers["Cache-Control"], "no-store")
        self.assertNotIn("Content-Length", rv.headers)
        self.assertEqual(self._body(rv), payload)

    def test_a_range_on_a_generated_stream_is_not_answered_with_a_fake_206(self):
        with open(self.track.path, "rb") as fh:
            payload = fh.read()
        os.remove(self.track.path)
        with self._live(payload):
            rv = self._get(headers={"Range": "bytes=1000-1999"})
        self.assertEqual(rv.status_code, 200)
        self.assertEqual(rv.headers["Accept-Ranges"], "none")
        self.assertEqual(self._body(rv), payload)


class TranscodeCacheIntegrityTestCase(_SeekCase):
    def test_a_transcode_ffmpeg_could_not_finish_is_not_cached(self):
        with open(self.track.path, "wb") as fh:
            fh.write(b"not audio at all" * 1000)
        from neezer.deezer import ids

        key = f"deezer-{ids.track_uuid('4242')}-opus128.ogg"
        with self.assertRaises(Exception):
            self._get("?q=OPUS_128")
        self.assertFalse(
            self.app.transcode_cache.has(key),
            "a failed transcode was published as the whole track",
        )

    def test_a_live_opus_transcode_cut_short_by_the_source_is_not_cached(self):
        from neezer.deezer import archive, ids

        with open(self.track.path, "rb") as fh:
            payload = fh.read(60000)
        os.remove(self.track.path)

        def live(provider, track, on_abort=None):
            def gen():
                yield payload[:30000]
                raise OSError("the provider connection died")

            return "audio/flac", gen()

        key = f"deezer-{ids.track_uuid('4242')}-opus128.ogg"
        with mock.patch.object(archive, "open_live_stream", live):
            rv = self._get("?q=OPUS_128")
            self.assertEqual(rv.status_code, 200)
            with self.assertRaises(Exception):
                self._body(rv)
        self.assertFalse(
            self.app.transcode_cache.has(key),
            "a live transcode of half a track was kept as the track",
        )


class ReadyFinalisationTestCase(_SeekCase):
    def test_ready_is_false_while_nothing_is_published_and_says_a_writer_is_at_work(self):
        os.remove(self.track.path)
        part = f"{self.track.path}.deadbeef.part"
        with open(part, "wb") as fh:
            fh.write(b"\0" * 100)
        payload = self.client.get("/api/stream/4242/ready").get_json()
        self.assertFalse(payload["ready"])
        self.assertTrue(payload["streaming"])
        self.assertIsNone(payload["bytes"])
        self.assertFalse(payload["offset"])

    def test_a_part_beside_the_published_archive_does_not_hide_it(self):
        # A concurrent download (or one left by a killed process) is visible as a
        # .part sibling. The file at the final name is published and complete, so
        # `ready` stays true and nothing even looks at the sibling.
        part = f"{self.track.path}.deadbeef.part"
        with open(part, "wb") as fh:
            fh.write(b"\0" * 100)
        payload = self.client.get("/api/stream/4242/ready").get_json()
        self.assertTrue(payload["ready"])
        self.assertFalse(payload["streaming"])
        self.assertEqual(payload["ranges"], "bytes")
        self.assertEqual(payload["bytes"], self.size)
        self.assertTrue(payload["offset"])
        os.remove(part)

    def test_an_opus_seek_is_offered_without_waiting_for_the_cache(self):
        payload = self.client.get("/api/stream/4242/ready?q=OPUS_128").get_json()
        self.assertFalse(payload["ready"], "no cache entry yet: not a file")
        self.assertFalse(payload["cached"])
        self.assertTrue(payload["offset"], "the archive is on disk: ?start= works")
        rv = self._get("?q=OPUS_128&start=2")
        self.assertEqual(rv.status_code, 200)
        self.assertEqual(rv.headers["X-NS-Start"], "2.000")

    def test_ready_costs_one_select(self):
        from .test_deezer import count_statements

        with count_statements() as counts:
            self.client.get("/api/stream/4242/ready")
        self.assertEqual(
            sum(counts.values()), 2, counts
        )


class EpisodeAndYoutubeReadyTestCase(_SeekCase):
    def test_a_podcast_episode_answers_whether_it_is_ready_instead_of_400(self):
        from neezer.db import PodcastChannel, PodcastEpisode, User

        user = User.get(User.name == "alice")
        channel = PodcastChannel.create(
            user=user, deezer_id="900", title="Show", url="https://example.test/feed"
        )
        path = os.path.join(self.archive, "Podcasts", "Show", "ep.mp3")
        os.makedirs(os.path.dirname(path), exist_ok=True)
        subprocess.run(
            [FFMPEG, "-v", "0", "-i", self.track.path, "-c:a", "libmp3lame",
             "-b:a", "96k", path, "-y"],
            check=True,
        )
        episode = PodcastEpisode.create(
            channel=channel, deezer_id="901", title="Ep", duration=12,
            path=path, status="completed",
        )
        payload = self.client.get(f"/api/stream/{episode.id}/ready").get_json()
        self.assertTrue(payload["ready"])
        self.assertEqual(payload["ranges"], "bytes")
        self.assertTrue(payload["offset"])

        payload = self.client.get(f"/api/stream/{episode.id}/ready?q=OPUS_64").get_json()
        self.assertFalse(payload["ready"], "the Opus transcode is not cached yet")
        self.assertTrue(payload["offset"])

        self.app.transcode_cache.set(f"episode-{episode.id}-opus64.ogg", b"OggS")
        payload = self.client.get(f"/api/stream/{episode.id}/ready?q=OPUS_64").get_json()
        self.assertTrue(payload["ready"])
        self.assertTrue(payload["cached"])

    def test_a_youtube_row_is_ready_as_a_file_when_no_transcode_is_needed(self):
        from neezer.deezer import library

        root = library.get_root_folder(self.archive)
        row = library.upsert_track(
            {"SNG_ID": "555", "SNG_TITLE": "Yt", "ART_ID": "1", "ART_NAME": "A",
             "ALB_ID": "2", "ALB_TITLE": "L", "DURATION": 12, "TRACK_NUMBER": 1,
             "DISK_NUMBER": 1}, root)
        yt_path = os.path.join(self.archive, "YouTube", "A", "L", "Yt.opus")
        os.makedirs(os.path.dirname(yt_path), exist_ok=True)
        shutil.copy(self.track.path, yt_path)
        row.path = yt_path
        row.youtube_id = "dQw4w9WgXcQ"
        row.deezer_id = None
        row.bitrate = 133
        row.save()
        payload = self.client.get(f"/api/stream/{row.id}/ready?q=OPUS_320").get_json()
        self.assertTrue(payload["ready"], "133 kbit/s is served as it is at OPUS_320")
        self.assertEqual(payload["ranges"], "bytes")

        payload = self.client.get(f"/api/stream/{row.id}/ready?q=OPUS_64").get_json()
        self.assertFalse(payload["ready"])
        self.assertTrue(payload["offset"])
        self.app.transcode_cache.set(f"youtube-{row.id}-opus64.ogg", b"OggS")
        payload = self.client.get(f"/api/stream/{row.id}/ready?q=OPUS_64").get_json()
        self.assertTrue(payload["ready"], "the key is youtube-…, not deezer-…")


    def test_an_mp4_upload_is_seeked_by_range_not_by_start(self):
        from neezer.db import User

        src = os.path.join(self.archive, "uploads", "u", "clip.m4a")
        os.makedirs(os.path.dirname(src), exist_ok=True)
        subprocess.run(
            [FFMPEG, "-v", "0", "-f", "lavfi", "-i",
             "sine=frequency=1000:duration=12", "-c:a", "aac", "-b:a", "96k",
             src, "-y"],
            check=True,
        )
        owner = User.get(User.name == "alice")
        row = Track.create(
            disc=1, number=1, title="Clip", duration=12, has_art=False,
            album=self.track.album, artist=self.track.artist, bitrate=96,
            path=src, owner=owner, last_modification=1,
            root_folder=self.track.root_folder, folder=self.track.folder,
        )
        payload = self.client.get(f"/api/stream/{row.id}/ready").get_json()
        self.assertTrue(payload["ready"])
        self.assertEqual(payload["ranges"], "bytes")
        self.assertFalse(
            payload["offset"], "an MP4 cannot be cut into a pipe: ranges only"
        )
        rv = self.client.get(f"/api/stream/{row.id}?start=5")
        self.assertEqual(rv.status_code, 409)
        self.assertEqual(rv.get_json()["ranges"], "bytes")


class SubsonicTimeOffsetTestCase(_SeekCase):
    transcoding = {
        "transcoder_flac_ogg":
            "ffmpeg -v 0 -i %srcpath -map 0:a:0 -c:a libopus -b:a %outratek "
            "-vbr on -f ogg -",
    }

    def setUp(self):
        super().setUp()
        self.app.config["TRANSCODING"] = dict(self.transcoding)

    def _stream(self, headers=None, **kwargs):
        auth = {"u": "alice", "p": "Alic3", "c": "t", "v": "1.12.0"}
        return self.client.get(
            "/rest/stream",
            query_string=dict(auth, id=str(self.track.id), **kwargs),
            headers=headers,
        )

    def test_a_subsonic_stream_answers_a_byte_range(self):
        with open(self.track.path, "rb") as fh:
            whole = fh.read()
        rv = self._stream()
        self.assertEqual(rv.status_code, 200)
        self.assertEqual(rv.headers["Accept-Ranges"], "bytes")
        self.assertEqual(rv.get_data(), whole)

        rv = self._stream(headers={"Range": "bytes=1000-2999"})
        self.assertEqual(rv.status_code, 206)
        self.assertEqual(rv.headers["Content-Range"], f"bytes 1000-2999/{self.size}")
        self.assertEqual(rv.get_data(), whole[1000:3000])

    def test_time_offset_starts_the_transcode_at_that_second(self):
        rv = self._stream(format="ogg", maxBitRate="128", timeOffset="9")
        self.assertEqual(rv.status_code, 200)
        self.assertEqual(rv.mimetype, "audio/ogg")
        self.assertEqual(rv.headers["X-NS-Start"], "9.000")
        self.assertEqual(rv.headers["Accept-Ranges"], "none")
        body = rv.get_data()
        self.assertTrue(body.startswith(b"OggS"))
        duration, leading = _decoded(body)
        self.assertAlmostEqual(duration, 3.0, delta=0.1)
        self.assertIsNone(leading)
        self.assertFalse(
            self.app.transcode_cache.has(f"{self.track.id}-128.ogg"),
            "a seek must not fill the whole-track cache entry",
        )
        self.assertEqual(
            Track[self.track.id].play_count, 0, "a seek is not a play"
        )

    def test_time_offset_ignores_the_whole_track_cache_entry(self):
        whole = self._stream(format="ogg", maxBitRate="128").get_data()
        self.assertAlmostEqual(_decoded(whole)[0], 12.0, delta=0.2)
        self.assertTrue(self.app.transcode_cache.has(f"{self.track.id}-128.ogg"))

        body = self._stream(format="ogg", maxBitRate="128", timeOffset="9").get_data()
        duration, leading = _decoded(body)
        self.assertAlmostEqual(
            duration, 3.0, delta=0.1, msg="the cached whole track was served, not a slice"
        )
        self.assertIsNone(leading)

    def test_a_junk_time_offset_is_a_subsonic_error(self):
        for bad in ("abc", "-5", "nan"):
            rv = self._stream(timeOffset=bad)
            self.assertEqual(rv.status_code, 200)
            self.assertIn(b"Invalid timeOffset", rv.get_data(), bad)

    def test_a_raw_stream_still_refuses_time_offset_rather_than_restarting(self):
        rv = self._stream(timeOffset="2")
        self.assertIn(b"Unsupported parameter", rv.get_data())


if __name__ == "__main__":
    unittest.main()
