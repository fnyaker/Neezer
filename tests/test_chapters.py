# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Chapters of an archived podcast episode (``neezer/deezer/chapters.py``).

The real shapes are produced by a real ffmpeg (an MP3 with ID3 CHAP/CTOC and an
M4A with a ``chpl`` box), the hostile ones by hand, and the API is driven on a
database the route has never seen. Skipped without ffmpeg — except where the
environment says it must be there (``NS_REQUIRE_FFMPEG=1``): a skip there would
be the suite quietly testing less than it claims.
"""

import collections
import contextlib
import json
import os
import shutil
import subprocess
import tempfile
import unittest
from unittest import mock

import peewee
import requests

from neezer.config import DefaultConfig
from neezer.db import (
    Meta,
    PodcastChannel,
    PodcastChapter,
    PodcastChapterScan,
    PodcastEpisode,
    User,
    db,
    init_database,
    release_database,
)
from neezer.deezer import chapters, ids, library, rules
from neezer.managers.user import UserManager
from neezer.web import create_application

HAVE_FFMPEG = shutil.which("ffmpeg") is not None

if not HAVE_FFMPEG and os.environ.get("NS_REQUIRE_FFMPEG") == "1":  # pragma: no cover
    raise RuntimeError("NS_REQUIRE_FFMPEG=1 but ffmpeg is not on PATH")


def show_data(show_id="1002156761", name="Test Podcast"):
    return {
        "SHOW_ID": show_id,
        "SHOW_NAME": name,
        "SHOW_DESCRIPTION": "A test podcast description.",
        "SHOW_ART_MD5": "showmd5",
        "SHOW_IS_DIRECT_STREAM": "1",
    }


def episode_obj(eid, title="Episode", duration=30, ts=1782660730):
    return {
        "EPISODE_ID": str(eid),
        "EPISODE_TITLE": title,
        "EPISODE_DESCRIPTION": "Episode description.",
        "DURATION": duration,
        "EPISODE_DIRECT_STREAM_URL": "https://host.example/ep.mp3",
        "EPISODE_IMAGE_MD5": "epmd5",
        "EPISODE_PUBLISHED_TS": ts,
        "AVAILABLE": True,
        "SHOW_ID": "1002156761",
        "SHOW_NAME": "Test Podcast",
        "SHOW_ART_MD5": "showmd5",
    }


def write_audio(path, payload=b"\xff\xfb\x90\x00" * 8):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as handle:
        handle.write(payload)
    return path


def write_id3(path, items, ctoc=None, url=None):
    """A real ID3 tag carrying CHAP frames (and an optional CTOC / URL tag)."""
    from mutagen.id3 import CHAP, CTOC, ID3, TIT2, TXXX

    write_audio(path)
    tag = ID3()
    for element_id, start, end, title in items:
        sub_frames = [TIT2(encoding=3, text=[title])] if title is not None else None
        tag.add(
            CHAP(
                element_id=element_id,
                start_time=start,
                end_time=end,
                start_offset=0xFFFFFFFF,
                end_offset=0xFFFFFFFF,
                sub_frames=sub_frames,
            )
        )
    if ctoc is not None:
        element_id, flags, children = ctoc
        tag.add(CTOC(element_id=element_id, flags=flags, child_element_ids=list(children)))
    if url:
        tag.add(TXXX(encoding=3, desc="podcast:chapters", text=[url]))
    tag.save(path)
    return path


def write_sidecar(path, document):
    sidecar = chapters.sidecar_path(path)
    with open(sidecar, "w", encoding="utf-8") as handle:
        if isinstance(document, str):
            handle.write(document)
        else:
            json.dump(document, handle)
    return sidecar


class RealFileChapterTestCase(unittest.TestCase):
    """The shapes the publishers' files really have, written by ffmpeg.

    Measured on the 240 kB/30 s fixtures: ID3 CHAP/CTOC 8.4 ms cold (mutagen's
    own import dominates; the tag lives at the head of the file), ``chpl``
    0.3 ms. Both are off the play path.
    """

    @classmethod
    def setUpClass(cls):
        cls.dir = tempfile.mkdtemp()
        cls.meta = os.path.join(cls.dir, "meta.txt")
        with open(cls.meta, "w", encoding="utf-8") as handle:
            handle.write(
                ";FFMETADATA1\n"
                "[CHAPTER]\nTIMEBASE=1/1000\nSTART=0\nEND=10000\ntitle=Introduction\n"
                "[CHAPTER]\nTIMEBASE=1/1000\nSTART=10000\nEND=25000\ntitle=L'invité\n"
                "[CHAPTER]\nTIMEBASE=1/1000\nSTART=25000\nEND=30000\ntitle=Questions\n"
            )
        cls.mp3 = cls._render("chap.mp3", ["-c:a", "libmp3lame", "-b:a", "64k"])
        cls.m4a = cls._render("chap.m4a", ["-c:a", "aac", "-b:a", "64k"])
        cls.plain = cls._render("plain.mp3", ["-c:a", "libmp3lame", "-b:a", "64k"], meta=False)

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.dir, ignore_errors=True)

    @classmethod
    def _render(cls, name, encoder, meta=True):
        base = os.path.join(cls.dir, "base-" + name)
        out = os.path.join(cls.dir, name)
        subprocess.run(
            [
                "ffmpeg", "-nostdin", "-v", "error", "-y",
                "-f", "lavfi", "-i", "sine=frequency=440:duration=30",
                *(encoder if meta else encoder), base,
            ],
            check=True,
            timeout=120,
        )
        if not meta:
            return base
        subprocess.run(
            [
                "ffmpeg", "-nostdin", "-v", "error", "-y",
                "-i", base, "-i", cls.meta,
                "-map_metadata", "1", "-map_chapters", "1", "-c", "copy", out,
            ],
            check=True,
            timeout=120,
        )
        return out

    def test_ffmpeg_wrote_the_two_shapes(self):
        from mutagen.id3 import ID3
        from mutagen.mp4 import MP4

        tag = ID3(self.mp3)
        self.assertEqual(len(tag.getall("CHAP")), 3)
        self.assertEqual(len(tag.getall("CTOC")), 1)
        self.assertEqual(len(MP4(self.m4a).chapters), 3)

    def test_the_id3_chapters_are_the_file_timeline(self):
        found, source = chapters.extract_embedded(self.mp3, duration_ms=30000)
        self.assertEqual(source, "id3")
        self.assertEqual(
            [(c.start_ms, c.end_ms, c.title) for c in found],
            [(0, 10000, "Introduction"), (10000, 25000, "L'invité"), (25000, 30000, "Questions")],
        )

    def test_the_mp4_chpl_is_read_as_seconds(self):
        found, source = chapters.extract_embedded(self.m4a, duration_ms=30000)
        self.assertEqual(source, "mp4")
        self.assertEqual([c.start_ms for c in found], [0, 10000, 25000])
        self.assertEqual([c.title for c in found], ["Introduction", "L'invité", "Questions"])

    def test_a_file_without_chapters_is_none(self):
        found, source = chapters.extract_embedded(self.plain, duration_ms=30000)
        self.assertEqual(found, [])
        self.assertEqual(source, "none")


class NormalizeTestCase(unittest.TestCase):
    def test_three_ordered_chapters_carry_the_file_timeline(self):
        found = chapters.normalize(
            [(0, None, " Introduction "), (10000, None, "L'invité"), (25000, None, "Questions")],
            duration_ms=30000,
            source="id3",
        )
        self.assertEqual(
            [(c.start_ms, c.end_ms, c.title) for c in found],
            [(0, 10000, "Introduction"), (10000, 25000, "L'invité"), (25000, 30000, "Questions")],
        )

    def test_a_chapter_cannot_start_after_a_day(self):
        found = chapters.normalize(
            [(25 * 3600 * 1000, None, "trop tard"), (0, None, "ok")],
            duration_ms=0,
            source="id3",
        )
        self.assertEqual([c.title for c in found], ["ok"])

    def test_negative_and_non_numeric_starts_are_dropped(self):
        found = chapters.normalize(
            [(-1, None, "neg"), ("x", None, "str"), (None, None, "none"),
             (float("nan"), None, "nan"), (True, None, "bool"), (0, None, "ok")],
            duration_ms=0,
            source="json",
        )
        self.assertEqual([c.title for c in found], ["ok"])

    def test_two_starts_within_a_second_keep_the_first(self):
        found = chapters.normalize(
            [(10000, None, "premier"), (10400, None, "doublon"), (12000, None, "troisième")],
            duration_ms=0,
            source="json",
        )
        self.assertEqual([c.title for c in found], ["premier", "troisième"])
        self.assertEqual(found[0].end_ms, 12000)

    def test_the_last_end_is_unknown_when_the_duration_is(self):
        found = chapters.normalize([(0, None, "a"), (10000, None, "b")], duration_ms=0, source="id3")
        self.assertIsNone(found[-1].end_ms)

    def test_a_given_end_is_cut_at_the_next_start(self):
        found = chapters.normalize(
            [(0, 60000, "a"), (10000, None, "b")], duration_ms=20000, source="id3"
        )
        self.assertEqual([c.end_ms for c in found], [10000, 20000])

    def test_titles_are_collapsed_capped_and_never_control_characters(self):
        found = chapters.normalize(
            [(0, None, "  a\n\tb  "), (10000, None, "x" * 1000), (20000, None, "  ")],
            duration_ms=0,
            source="json",
        )
        self.assertEqual(found[0].title, "a b")
        self.assertEqual(len(found[1].title), chapters.MAX_TITLE)
        self.assertIsNone(found[2].title)

    def test_the_cap_bounds_a_hostile_file(self):
        found = chapters.normalize(
            [(i * 2000, None, f"c{i}") for i in range(700)], duration_ms=0, source="id3"
        )
        self.assertEqual(len(found), chapters.MAX_CHAPTERS)

    def test_the_json_document_drops_toc_false_and_keeps_the_order(self):
        document = {
            "version": "1.2.0",
            "chapters": [
                {"startTime": 0, "title": "Intro"},
                {"startTime": 60, "title": "secret", "toc": False},
                {"startTime": 120.5, "title": "Fin", "endTime": 200},
            ],
        }
        raw = chapters.parse_json_chapters(json.dumps(document).encode())
        self.assertEqual([item[2] for item in raw], ["Intro", "Fin"])
        self.assertEqual([item[0] for item in raw], [0, 120500])
        self.assertEqual(raw[1][1], 200000)

    def test_a_bare_array_is_the_same_document(self):
        raw = chapters.parse_json_chapters('[{"startTime": 5, "title": "A"}]')
        self.assertEqual(raw, [(5000, None, "A")])

    def test_anything_that_is_not_the_document_raises(self):
        for payload in (b"<html>nope</html>", '{"chapters": 3}', "[]" * 5, b"\xff\xfe\x00"):
            with self.assertRaises(ValueError, msg=payload):
                chapters.parse_json_chapters(payload)


class EmbeddedChapterTestCase(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.dir, True)

    def path(self, name="ep.mp3"):
        return os.path.join(self.dir, "Podcasts", "Show", name)

    def test_no_ctoc_falls_back_to_the_chap_starts(self):
        from mutagen.id3 import CTOCFlags

        path = write_id3(
            self.path(),
            [("b", 20000, 0xFFFFFFFF, "Deux"), ("a", 5000, 9000, "Un")],
            ctoc=("toc", CTOCFlags.ORDERED, ["b", "a"]),
        )
        found, source = chapters.extract_embedded(path, duration_ms=30000)
        self.assertEqual(source, "id3")
        self.assertEqual([(c.start_ms, c.end_ms, c.title) for c in found],
                         [(5000, 9000, "Un"), (20000, 30000, "Deux")])

    def test_a_top_level_ctoc_gives_the_order(self):
        from mutagen.id3 import CTOCFlags

        path = write_id3(
            self.path(),
            [("a", 5000, 9000, "Un"), ("b", 20000, 25000, "Deux")],
            ctoc=("toc", CTOCFlags.TOP_LEVEL | CTOCFlags.ORDERED, ["b", "a"]),
        )
        found, source = chapters.extract_embedded(path, duration_ms=30000)
        self.assertEqual(source, "id3")
        self.assertEqual([c.title for c in found], ["Un", "Deux"])

    def test_a_nested_ctoc_is_flattened_and_stops_at_the_depth_cap(self):
        from mutagen.id3 import CTOC, CTOCFlags

        path = write_id3(
            self.path(),
            [("c0", 0, 5000, "zéro"), ("deep", 10000, 15000, "profond")],
            ctoc=("toc", CTOCFlags.TOP_LEVEL | CTOCFlags.ORDERED, ["c0", "s1"]),
        )
        from mutagen.id3 import ID3

        tag = ID3(path)
        tag.add(CTOC(element_id="s1", flags=CTOCFlags.ORDERED, child_element_ids=["s2"]))
        tag.add(CTOC(element_id="s2", flags=CTOCFlags.ORDERED, child_element_ids=["s3"]))
        tag.add(CTOC(element_id="s3", flags=CTOCFlags.ORDERED, child_element_ids=["s4"]))
        tag.add(CTOC(element_id="s4", flags=CTOCFlags.ORDERED, child_element_ids=["deep"]))
        tag.save(path)
        found, source = chapters.extract_embedded(path, duration_ms=30000)
        self.assertEqual(source, "id3")
        self.assertEqual([c.title for c in found], ["zéro"])

    def test_a_ctoc_cycle_does_not_hang(self):
        from mutagen.id3 import CTOC, CTOCFlags

        path = write_id3(
            self.path(),
            [("c0", 0, 5000, "zéro")],
            ctoc=("toc", CTOCFlags.TOP_LEVEL | CTOCFlags.ORDERED, ["toc", "c0", "loop"]),
        )
        from mutagen.id3 import ID3

        tag = ID3(path)
        tag.add(CTOC(element_id="loop", flags=CTOCFlags.ORDERED, child_element_ids=["toc"]))
        tag.save(path)
        found, source = chapters.extract_embedded(path, duration_ms=30000)
        self.assertEqual(source, "id3")
        self.assertEqual([c.title for c in found], ["zéro"])

    def test_a_huge_id3_tag_is_refused_before_reading_it(self):
        path = self.path()
        size = 11 * 1024 * 1024
        head = bytes([(size >> 21) & 0x7F, (size >> 14) & 0x7F, (size >> 7) & 0x7F, size & 0x7F])
        write_audio(path, b"ID3\x03\x00\x00" + head + b"\x00" * 64)
        found, source = chapters.extract_embedded(path, duration_ms=30000)
        self.assertEqual(found, [])
        self.assertEqual(source, "none")

    def test_a_corrupt_tag_is_none_and_never_raises(self):
        path = self.path()
        write_audio(path, b"ID3\x03\x00\x00\x00\x00\x02\x00" + b"junk" * 20)
        found, source = chapters.extract_embedded(path, duration_ms=30000)
        self.assertEqual((found, source), ([], "none"))

    def test_a_file_that_is_not_audio_is_none_and_never_raises(self):
        path = write_audio(self.path(), b"<html><body>consent</body></html>")
        found, source = chapters.extract_embedded(path, duration_ms=30000)
        self.assertEqual((found, source), ([], "none"))

    def test_the_publisher_url_is_read_from_txxx_wxxx_and_mp4_freeform(self):
        from mutagen.id3 import ID3, WXXX

        path = write_id3(self.path(), [], url="https://publisher.example/chapters.json")
        self.assertEqual(chapters.url_from_tags(path), "https://publisher.example/chapters.json")

        other = self.path("wxxx.mp3")
        write_audio(other)
        tag = ID3()
        tag.add(WXXX(encoding=3, desc="chapters", url="https://publisher.example/b.json"))
        tag.save(other)
        self.assertEqual(chapters.url_from_tags(other), "https://publisher.example/b.json")

        class StubMp4:
            tags = {"----:com.apple.iTunes:chapters": [b"https://publisher.example/c.json"]}

        untagged = write_audio(self.path("untagged.mp3"))
        with mock.patch("mutagen.mp4.MP4", lambda path: StubMp4()):
            self.assertEqual(
                chapters.url_from_tags(untagged), "https://publisher.example/c.json"
            )

    def test_a_url_tag_that_is_not_http_is_ignored(self):
        path = write_id3(self.path(), [], url="file:///etc/passwd")
        self.assertIsNone(chapters.url_from_tags(path))


class SidecarTestCase(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.dir, True)
        self.episode = os.path.join(self.dir, "Podcasts", "Show", "2026-01-01 - Ep.mp3")

    def test_a_podcasting_document_wins_over_the_embedded_tags(self):
        write_id3(self.episode, [("a", 0, 5000, "tag"), ("b", 10000, 15000, "tag2")])
        write_sidecar(
            self.episode,
            {"version": "1.2.0", "chapters": [{"startTime": 3, "title": "Curé"}]},
        )
        found, source, error = chapters.extract(self.episode, duration_ms=30000, allow_url=False)
        self.assertEqual((source, error), ("sidecar", None))
        self.assertEqual([(c.start_ms, c.title) for c in found], [(3000, "Curé")])

    def test_a_bare_array_is_a_sidecar_too(self):
        write_audio(self.episode)
        write_sidecar(self.episode, [{"startTime": 1}, {"startTime": 20, "title": "Fin"}])
        found, source, error = chapters.extract(self.episode, duration_ms=30000, allow_url=False)
        self.assertEqual(source, "sidecar")
        self.assertEqual([c.start_ms for c in found], [1000, 20000])

    def test_an_explicit_empty_sidecar_means_no_chapters(self):
        write_id3(self.episode, [("a", 0, 5000, "tag")])
        write_sidecar(self.episode, {"version": "1.2.0", "chapters": []})
        found, source, error = chapters.extract(self.episode, duration_ms=30000, allow_url=False)
        self.assertEqual((found, source, error), ([], "none", None))

    def test_an_invalid_sidecar_is_recorded_and_the_tags_are_still_read(self):
        write_id3(self.episode, [("a", 0, 5000, "tag")])
        write_sidecar(self.episode, "{not json")
        found, source, error = chapters.extract(self.episode, duration_ms=30000, allow_url=False)
        self.assertEqual(source, "id3")
        self.assertEqual([c.title for c in found], ["tag"])
        self.assertIn("invalid sidecar", error)

    def test_a_sidecar_over_the_cap_is_refused(self):
        write_audio(self.episode)
        sidecar = chapters.sidecar_path(self.episode)
        with open(sidecar, "w", encoding="utf-8") as handle:
            handle.write(" " * (chapters.MAX_SIDECAR_BYTES + 10))
        found, source, error = chapters.extract(self.episode, duration_ms=0, allow_url=False)
        self.assertEqual((found, source), ([], "none"))
        self.assertEqual(error, "sidecar too large")

    def test_a_sidecar_reaching_outside_the_archive_is_refused(self):
        write_audio(self.episode)
        outside = os.path.join(self.dir, "elsewhere.json")
        with open(outside, "w", encoding="utf-8") as handle:
            json.dump({"chapters": [{"startTime": 1, "title": "hors"}]}, handle)
        try:
            os.symlink(outside, chapters.sidecar_path(self.episode))
        except (OSError, NotImplementedError):
            self.skipTest("symlinks are not available here")
        found, source, error = chapters.extract(self.episode, duration_ms=0, allow_url=False)
        self.assertEqual((found, source), ([], "none"))
        self.assertEqual(error, "sidecar outside the podcast archive")

    def test_a_directory_named_like_a_sidecar_is_refused(self):
        write_audio(self.episode)
        os.makedirs(chapters.sidecar_path(self.episode))
        found, source, error = chapters.extract(self.episode, duration_ms=0, allow_url=False)
        self.assertEqual((found, source), ([], "none"))
        self.assertEqual(error, "sidecar is not a regular file")


class FakeResponse:
    def __init__(self, status_code=200, headers=None, body=b"", chunks=None):
        self.status_code = status_code
        self.headers = requests_headers(headers or {})
        self._body = body
        self._chunks = chunks

    @property
    def is_redirect(self):
        return 300 <= self.status_code < 400

    is_permanent_redirect = is_redirect

    def raise_for_status(self):
        if self.status_code >= 400:
            raise requests.HTTPError(f"{self.status_code}")

    def iter_content(self, size):
        if self._chunks is not None:
            yield from self._chunks
            return
        for start in range(0, len(self._body), size):
            yield self._body[start : start + size]

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        return False


def requests_headers(values):
    from requests.structures import CaseInsensitiveDict

    return CaseInsensitiveDict(values)


class FakeSession:
    def __init__(self, routes, log):
        self.routes = routes
        self.log = log
        self.closed = False

    def mount(self, prefix, adapter):
        self.log.append(("pin", adapter.address))

    def get(self, url, **kwargs):
        self.log.append(("get", url, dict(kwargs.get("headers") or {})))
        return self.routes[url]

    def close(self):
        self.closed = True
        self.log.append(("close",))


class UrlFetchTestCase(unittest.TestCase):
    """A fake pinned session: the guard, the hops and the caps, with no network."""

    def setUp(self):
        self.log = []
        self.routes = {}
        self.sessions = []
        self.validated = []

        def new_session():
            session = FakeSession(self.routes, self.log)
            self.sessions.append(session)
            return session

        def check(url):
            self.validated.append(url)
            return ["93.184.216.34"]

        for name, value in (
            ("new_session", new_session),
            ("check_public_url", check),
            ("select_proxy", lambda url, proxies: None),
        ):
            patcher = mock.patch.object(chapters, name, value)
            patcher.start()
            self.addCleanup(patcher.stop)

    def test_every_hop_is_validated_pinned_and_cookieless(self):
        first = "https://publisher.example/chapters.json"
        second = "https://cdn.example/chapters.json"
        self.routes[first] = FakeResponse(
            302, {"Location": second, "Set-Cookie": "tracker=1"}
        )
        self.routes[second] = FakeResponse(
            200,
            {"Content-Type": "application/json"},
            body=json.dumps({"chapters": [{"startTime": 0, "title": "Intro"}]}).encode(),
        )
        raw, error = chapters.fetch_json_chapters(first)
        self.assertIsNone(error)
        self.assertEqual(raw, [(0, None, "Intro")])
        self.assertEqual(self.validated, [first, second])
        pins = [entry[1] for entry in self.log if entry[0] == "pin"]
        self.assertEqual(pins, ["93.184.216.34"] * 4)
        self.assertEqual(len(self.sessions), 2)
        self.assertTrue(all(session.closed for session in self.sessions))
        for entry in self.log:
            if entry[0] == "get":
                self.assertNotIn("Cookie", {k.lower() for k in entry[2]})
                self.assertIn("Host", entry[2])

    def test_a_private_address_is_refused(self):
        from neezer.deezer.provider import DeezerError

        def refuse(url):
            raise DeezerError("refusing episode URL resolving to 127.0.0.1")

        with mock.patch.object(chapters, "check_public_url", refuse):
            raw, error = chapters.fetch_json_chapters("http://localhost/chapters.json")
        self.assertIsNone(raw)
        self.assertIn("127.0.0.1", error)
        self.assertEqual(self.sessions, [])

    def test_an_html_answer_is_refused(self):
        url = "https://publisher.example/chapters.json"
        self.routes[url] = FakeResponse(
            200, {"Content-Type": "text/html"}, body=b"<!doctype html><p>nope</p>"
        )
        raw, error = chapters.fetch_json_chapters(url)
        self.assertIsNone(raw)
        self.assertIn("not JSON", error)

    def test_a_document_over_the_cap_is_refused(self):
        url = "https://publisher.example/chapters.json"
        body = b'{"chapters":[' + b" " * chapters.MAX_JSON_BYTES + b"]}"
        self.routes[url] = FakeResponse(200, {"Content-Type": "application/json"}, body=body)
        raw, error = chapters.fetch_json_chapters(url)
        self.assertIsNone(raw)
        self.assertEqual(error, "chapters document too large")

    def test_four_redirects_are_too_many(self):
        host = "https://publisher.example/"
        for step in range(4):
            self.routes[host + str(step)] = FakeResponse(
                302, {"Location": host + str(step + 1)}
            )
        self.routes[host + "4"] = FakeResponse(
            200, {"Content-Type": "application/json"}, body=b'{"chapters": []}'
        )
        raw, error = chapters.fetch_json_chapters(host + "0")
        self.assertIsNone(raw)
        self.assertEqual(error, "too many redirects")

    def test_a_redirect_without_a_location_is_refused(self):
        url = "https://publisher.example/chapters.json"
        self.routes[url] = FakeResponse(302, {})
        raw, error = chapters.fetch_json_chapters(url)
        self.assertIsNone(raw)
        self.assertEqual(error, "redirect without a Location")


class PodcastAppTestCase(unittest.TestCase):
    def setUp(self):
        self.fd, self.db_path = tempfile.mkstemp()
        self.tmp = tempfile.mkdtemp()
        self.archive = os.path.join(self.tmp, "archive")
        os.makedirs(self.archive)
        db_path = self.db_path
        cache = os.path.join(self.tmp, "cache")
        archive = self.archive

        class Config(DefaultConfig):
            TESTING = True

            def __init__(self):
                super().__init__()
                self.BASE = dict(self.BASE, database_uri="sqlite:///" + db_path)
                self.WEBAPP = dict(
                    self.WEBAPP, cache_dir=cache, mount_webui=True, mount_api=True
                )
                self.DEEZER = dict(getattr(self, "DEEZER", {}), archive_dir=archive)

        self.app = create_application(Config())
        rules.invalidate()
        self.addCleanup(rules.invalidate)
        UserManager.add("alice", "Alic3", admin=True)
        self.user = User.get(User.name == "alice")
        self.client = self.app.test_client()

    def tearDown(self):
        from neezer.deezer import chapters as module

        with module._queue_lock:
            module._queue.clear()
        release_database()
        shutil.rmtree(self.tmp, ignore_errors=True)
        os.close(self.fd)
        try:
            os.remove(self.db_path)
        except OSError:
            pass

    def login(self):
        self.client.post("/api/login", json={"username": "alice", "password": "Alic3"})

    def channel(self):
        return library.upsert_channel(self.user, library.normalize_show(show_data()))

    def episode(self, number=1, duration=30, archived=False, name=None):
        channel = self.channel()
        episode = library.upsert_episode(
            channel,
            library.normalize_episode(
                episode_obj(number, name or f"Ep {number}", duration=duration)
            ),
        )
        if archived:
            path = library.episode_archive_path(
                self.archive, channel.title, episode.publish_date, episode.title
            )
            write_audio(path)
            episode.path = path
            episode.status = "completed"
            episode.save()
        return episode


class ChaptersApiTestCase(PodcastAppTestCase):
    def setUp(self):
        super().setUp()
        self.login()

    def url(self, episode):
        return "/api/podcast/episode/" + str(episode.id) + "/chapters"

    def test_a_logged_out_client_is_unauthorized(self):
        episode = self.episode(archived=True)
        self.client.post("/api/logout")
        self.assertEqual(self.client.get(self.url(episode)).status_code, 401)

    def test_an_unknown_episode_is_a_404_with_a_code(self):
        for value in (str(ids.episode_uuid("9")), "not-a-uuid", "0" * 5000):
            response = self.client.get("/api/podcast/episode/" + value + "/chapters")
            self.assertEqual(response.status_code, 404, value)
            self.assertEqual(response.get_json(), {"error": "not found", "code": "not_found"})

    def test_an_episode_that_is_not_archived_is_a_200(self):
        episode = self.episode(archived=False)
        body = self.client.get(self.url(episode)).get_json()
        self.assertEqual(
            body,
            {
                "episode_id": str(episode.id),
                "archived": False,
                "scanned": False,
                "source": None,
                "duration": 30,
                "chapters": [],
            },
        )

    def test_the_first_open_extracts_and_the_warm_answer_is_two_statements(self):
        episode = self.episode(archived=True)
        write_id3(
            episode.path,
            [("a", 0, 10000, "Introduction"), ("b", 10000, 25000, "L'invité")],
        )
        body = self.client.get(self.url(episode)).get_json()
        self.assertTrue(body["archived"])
        self.assertTrue(body["scanned"])
        self.assertEqual(body["source"], "id3")
        self.assertEqual(
            body["chapters"],
            [
                {"index": 0, "start": 0, "end": 10, "title": "Introduction"},
                {"index": 1, "start": 10, "end": 25, "title": "L'invité"},
            ],
        )
        with count_statements() as counts:
            again = self.client.get(self.url(episode)).get_json()
        self.assertEqual(again, body)
        # The session, the scan read and the chapter rows — the route's own
        # work is the two the plan budgets.
        self.assertEqual(counts["SELECT"], 3, counts)

    def test_a_corrupt_tag_is_a_200_with_none_and_scanned(self):
        episode = self.episode(archived=True)
        write_audio(episode.path, b"ID3\x03\x00\x00\x00\x00\x02\x00" + b"junk" * 20)
        body = self.client.get(self.url(episode)).get_json()
        self.assertEqual(body["source"], "none")
        self.assertTrue(body["scanned"])
        self.assertEqual(body["chapters"], [])
        self.assertTrue(PodcastChapterScan.select().where(
            PodcastChapterScan.episode == episode.id).exists())

    def test_an_empty_sidecar_wins_over_the_tags(self):
        episode = self.episode(archived=True)
        write_id3(episode.path, [("a", 0, 5000, "tag")])
        write_sidecar(episode.path, {"chapters": []})
        body = self.client.get(self.url(episode)).get_json()
        self.assertEqual(body["source"], "none")
        self.assertEqual(body["chapters"], [])
        self.assertEqual(PodcastChapter.select().count(), 0)

    def test_the_extraction_switch_off_leaves_the_episode_unscanned(self):
        episode = self.episode(archived=True)
        write_id3(episode.path, [("a", 0, 5000, "tag")])
        rules.save({"chapters": False})
        try:
            body = self.client.get(self.url(episode)).get_json()
        finally:
            rules.save({"chapters": True})
        self.assertEqual(body["source"], None)
        self.assertFalse(body["scanned"])
        self.assertEqual(body["chapters"], [])
        self.assertFalse(PodcastChapterScan.select().exists())

    def test_the_payload_is_in_seconds_and_titles_are_bounded(self):
        episode = self.episode(archived=True, duration=3600)
        write_id3(episode.path, [("a", 612000, 2401000, "x" * 1000)])
        body = self.client.get(self.url(episode)).get_json()
        chapter = body["chapters"][0]
        self.assertEqual(chapter["start"], 612)
        self.assertEqual(chapter["end"], 2401)
        self.assertEqual(len(chapter["title"]), chapters.MAX_TITLE)
        self.assertEqual(body["duration"], 3600)

    def test_the_cap_bounds_the_rows_and_the_payload(self):
        episode = self.episode(archived=True, duration=7200)
        write_id3(
            episode.path,
            [(f"c{i}", i * 2000, 0xFFFFFFFF, f"c{i}") for i in range(600)],
        )
        body = self.client.get(self.url(episode)).get_json()
        self.assertEqual(len(body["chapters"]), chapters.MAX_CHAPTERS)
        self.assertEqual(PodcastChapter.select().count(), chapters.MAX_CHAPTERS)

    def test_the_show_payload_carries_the_count_not_the_list(self):
        channel = self.channel()
        first = library.upsert_episode(
            channel, library.normalize_episode(episode_obj(1, "Un"))
        )
        second = library.upsert_episode(
            channel, library.normalize_episode(episode_obj(2, "Deux"))
        )
        for episode, count in ((first, 2), (second, 1)):
            path = library.episode_archive_path(
                self.archive, channel.title, episode.publish_date, episode.title
            )
            write_audio(path)
            episode.path = path
            episode.save()
            write_sidecar(
                path, {"chapters": [{"startTime": index * 5} for index in range(count)]}
            )
            chapters.ensure(episode)

        with count_statements() as counts:
            body = self.client.get("/api/podcast/" + str(channel.id)).get_json()
        by_id = {episode["id"]: episode for episode in body["episodes"]}
        self.assertEqual(by_id[str(first.id)]["chapter_count"], 2)
        self.assertEqual(by_id[str(second.id)]["chapter_count"], 1)
        self.assertNotIn("chapters", by_id[str(first.id)])
        # The session, the channel, its ONE grouped counts read (episodes and
        # archived in a single query), its episodes, ONE grouped chapter count
        # and ONE grouped transcription state.
        self.assertEqual(counts["SELECT"], 6, counts)

    def test_a_show_with_no_chapters_says_zero(self):
        channel = self.channel()
        library.upsert_episode(channel, library.normalize_episode(episode_obj(1, "Un")))
        body = self.client.get("/api/podcast/" + str(channel.id)).get_json()
        self.assertEqual(body["episodes"][0]["chapter_count"], 0)


class UrlChapterApiTestCase(PodcastAppTestCase):
    def setUp(self):
        super().setUp()
        self.login()
        self.ep = self.episode(archived=True)
        write_id3(self.ep.path, [], url="https://publisher.example/c.json")

    def test_the_switch_is_off_by_default(self):
        self.assertFalse(rules.DEFAULTS["chapter_urls"])
        self.assertTrue(rules.DEFAULTS["chapters"])
        self.assertFalse(rules.load(self.app)["chapter_urls"])

    def test_the_url_is_not_followed_while_the_switch_is_off(self):
        with mock.patch.object(chapters, "fetch_json_chapters") as fetch, \
             self.app.app_context():
            self.assertFalse(chapters.enqueue_url(self.ep))
            chapters.drain_urls()
        fetch.assert_not_called()

    def test_the_url_is_followed_when_the_switch_is_on(self):
        url = "https://publisher.example/c.json"
        body = json.dumps(
            {"chapters": [{"startTime": 0, "title": "Intro"}, {"startTime": 15, "title": "Fin"}]}
        ).encode()
        routes = {url: FakeResponse(200, {"Content-Type": "application/json"}, body=body)}
        rules.save({"chapter_urls": True})
        try:
            with self.app.app_context(), \
                 mock.patch.object(chapters, "check_public_url", lambda u: ["93.184.216.34"]), \
                 mock.patch.object(chapters, "select_proxy", lambda u, p: None), \
                 mock.patch.object(
                     chapters,
                     "new_session",
                     lambda: FakeSession(routes, []),
                 ):
                self.assertTrue(chapters.enqueue_url(self.ep))
                self.assertEqual(chapters.drain_urls(), 1)
        finally:
            rules.save({"chapter_urls": False})
        scan = PodcastChapterScan.get(PodcastChapterScan.episode == self.ep.id)
        self.assertEqual((scan.source, scan.n_chapters), ("json", 2))
        self.assertEqual(
            [(c.start_ms, c.title) for c in PodcastChapter.select().order_by(PodcastChapter.idx)],
            [(0, "Intro"), (15000, "Fin")],
        )

    def test_a_failed_fetch_is_recorded_and_never_fails_the_episode(self):
        rules.save({"chapter_urls": True})
        try:
            with self.app.app_context(), mock.patch.object(
                chapters, "check_public_url", side_effect=requests.RequestException("nope")
            ):
                self.assertTrue(chapters.enqueue_url(self.ep))
                self.assertEqual(chapters.drain_urls(), 1)
        finally:
            rules.save({"chapter_urls": False})
        body = self.client.get(
            "/api/podcast/episode/" + str(self.ep.id) + "/chapters"
        ).get_json()
        self.assertTrue(body["archived"])
        self.assertEqual(body["chapters"], [])
        scan = PodcastChapterScan.get(PodcastChapterScan.episode == self.ep.id)
        self.assertEqual(scan.source, "none")
        self.assertTrue(scan.error)

    def test_the_explicit_scan_follows_the_url_inline(self):
        url = "https://publisher.example/c.json"
        body = json.dumps({"chapters": [{"startTime": 7, "title": "Sept"}]}).encode()
        routes = {url: FakeResponse(200, {"Content-Type": "application/json"}, body=body)}
        with self.app.app_context(), \
             mock.patch.object(chapters, "check_public_url", lambda u: ["93.184.216.34"]), \
             mock.patch.object(chapters, "select_proxy", lambda u, p: None), \
             mock.patch.object(chapters, "new_session", lambda: FakeSession(routes, [])):
            self.assertEqual(chapters.ensure(self.ep, force=True, allow_url=True), 1)
        scan = PodcastChapterScan.get(PodcastChapterScan.episode == self.ep.id)
        self.assertEqual((scan.source, scan.n_chapters), ("json", 1))
        self.assertEqual(PodcastChapter.get(PodcastChapter.episode == self.ep.id).start_ms, 7000)

    def test_a_stale_empty_scan_re_queues_the_fetch_once(self):
        from datetime import timedelta

        from neezer.db import now

        rules.save({"chapter_urls": True})
        try:
            PodcastChapterScan.delete().where(
                PodcastChapterScan.episode == self.ep.id
            ).execute()
            fresh = PodcastChapterScan.create(
                episode=self.ep.id, scanned_at=now(), source="none", n_chapters=0
            )
            url = "/api/podcast/episode/" + str(self.ep.id) + "/chapters"
            with mock.patch.object(chapters, "enqueue_url") as enqueue, \
                 self.app.app_context():
                self.client.get(url)
            enqueue.assert_not_called()

            fresh.scanned_at = now() - timedelta(days=8)
            fresh.save()
            with mock.patch.object(chapters, "enqueue_url") as enqueue, \
                 self.app.app_context():
                self.client.get(url)
            enqueue.assert_called_once()
        finally:
            rules.save({"chapter_urls": False})

    def test_the_finalize_hook_queues_the_fetch_once(self):
        rules.save({"chapter_urls": True})
        try:
            with self.app.app_context():
                self.assertTrue(chapters.enqueue_url(self.ep))
                self.assertTrue(chapters.enqueue_url(self.ep))
        finally:
            rules.save({"chapter_urls": False})
        with chapters._queue_lock:
            self.assertEqual(len(chapters._queue), 1)


class BackfillTestCase(PodcastAppTestCase):
    def setUp(self):
        super().setUp()
        self.archived = self.episode(1, archived=True)
        write_id3(self.archived.path, [("a", 0, 5000, "tag")])
        self.other = self.episode(2, archived=True)
        write_audio(self.other.path)
        self.missing = self.episode(3, archived=False)

    def test_the_boot_backfill_scans_only_archived_unscanned_episodes(self):
        job = chapters.start_backfill(self.app)
        self.assertIsNotNone(job)
        stats = job.run()
        self.assertEqual(stats["done"], 2)
        self.assertEqual(stats["failed"], 0)
        self.assertEqual(
            {str(row.episode_id) for row in PodcastChapterScan.select()},
            {str(self.archived.id), str(self.other.id)},
        )
        self.assertFalse(
            PodcastChapterScan.select()
            .where(PodcastChapterScan.episode == self.missing.id)
            .exists()
        )
        self.assertIsNone(chapters.start_backfill(self.app))

    def test_a_second_run_does_not_scan_again(self):
        chapters.start_backfill(self.app).run()
        stamp = PodcastChapterScan.get(
            PodcastChapterScan.episode == self.archived.id
        ).scanned_at
        with mock.patch.object(chapters, "enabled", lambda: True):
            with count_statements() as counts:
                self.assertIsNone(chapters.start_backfill(self.app))
        # One EXISTS: a fully scanned archive costs one query at boot.
        self.assertEqual(counts["SELECT"], 1, counts)
        self.assertEqual(
            PodcastChapterScan.get(PodcastChapterScan.episode == self.archived.id).scanned_at,
            stamp,
        )

    def test_a_ghost_archive_is_reported_and_left_for_the_next_boot(self):
        os.unlink(self.other.path)
        job = chapters.start_backfill(self.app)
        stats = job.run()
        self.assertEqual(stats["done"], 1)
        self.assertEqual(stats["failed"], 1)
        self.assertIn("archive missing", stats["failure_reasons"])
        self.assertFalse(
            PodcastChapterScan.select()
            .where(PodcastChapterScan.episode == self.other.id)
            .exists()
        )
        self.assertIsNotNone(chapters.start_backfill(self.app))

    def test_the_boot_backfill_runs_on_its_own_thread_outside_tests(self):
        import threading

        started = threading.Event()

        class FakeApp:
            testing = False

            def app_context(self):
                return contextlib.nullcontext()

        job = mock.Mock()
        job.run.side_effect = lambda: started.set()

        with mock.patch.object(chapters, "enabled", lambda: True), \
             mock.patch.object(chapters, "_pending", lambda: True), \
             mock.patch.object(chapters, "make_job", lambda *a, **k: job):
            self.assertIs(chapters.start_backfill(FakeApp()), job)
            self.assertTrue(started.wait(5), "the boot walk never started")
        job.run.assert_called_once()

    def test_the_job_is_resumable_from_its_cursor(self):
        job = chapters.make_job()
        job.page_size = 1
        job.run(limit=1)
        self.assertIsNotNone(Meta.get_or_none(Meta.key == chapters.CURSOR_KEY))
        rest = chapters.make_job().run()
        self.assertEqual(rest["done"], 1)
        self.assertIsNone(Meta.get_or_none(Meta.key == chapters.CURSOR_KEY))


class UpgradeTestCase(unittest.TestCase):
    """The chapter tables come back on a database that predates them, and the
    podcast rows and their audio survive — twice, without duplicating a row."""

    def setUp(self):
        self.fd, self.path = tempfile.mkstemp()
        self.uri = "sqlite:///" + self.path
        self.tmp = tempfile.mkdtemp()

    def tearDown(self):
        release_database()
        shutil.rmtree(self.tmp, ignore_errors=True)
        os.close(self.fd)
        try:
            os.remove(self.path)
        except OSError:
            pass

    def _sql(self, *statements):
        import sqlite3

        connection = sqlite3.connect(self.path)
        for statement in statements:
            connection.execute(statement)
        connection.commit()
        connection.close()

    def test_an_old_database_gains_the_chapter_tables_and_keeps_its_rows(self):
        init_database(self.uri)
        user = UserManager.add("alice", "Alic3", admin=True)
        channel = library.upsert_channel(user, library.normalize_show(show_data()))
        episode = library.upsert_episode(
            channel, library.normalize_episode(episode_obj(1, "Ep 1"))
        )
        path = os.path.join(self.tmp, "Podcasts", "Test Podcast", "ep.mp3")
        write_audio(path)
        episode.path = path
        episode.status = "completed"
        episode.save()
        release_database()

        self._sql(
            "DROP TABLE podcast_chapter_scan",
            "DROP TABLE podcast_chapter",
            "UPDATE meta SET value = '20261115' WHERE key = 'schema_version'",
        )

        for _run in range(2):
            init_database(self.uri)
            self.assertTrue(db.table_exists("podcast_chapter"))
            self.assertTrue(db.table_exists("podcast_chapter_scan"))
            self.assertEqual(PodcastChapter.select().count(), 0)
            self.assertEqual(PodcastChapterScan.select().count(), 0)
            kept = PodcastEpisode.get(PodcastEpisode.id == episode.id)
            self.assertEqual(kept.path, path)
            self.assertTrue(os.path.isfile(kept.path))
            self.assertEqual(PodcastChannel.select().count(), 1)
            release_database()


class ArchiveHookTestCase(PodcastAppTestCase):
    """The archive finalize publishes the file, then reads its own chapters."""

    def path_for(self, episode):
        return library.episode_archive_path(
            self.archive, episode.channel.title, episode.publish_date, episode.title
        )

    def test_finalizing_an_archive_extracts_its_chapters(self):
        from neezer.deezer import archive

        episode = self.episode(archived=False)
        path = self.path_for(episode)
        write_id3(path, [("a", 0, 5000, "Un"), ("b", 5000, 20000, "Deux")])
        archive._finalize_episode(episode, path)
        scan = PodcastChapterScan.get(PodcastChapterScan.episode == episode.id)
        self.assertEqual((scan.source, scan.n_chapters), ("id3", 2))

    def test_the_switch_off_keeps_finalize_from_reading_the_file(self):
        from neezer.deezer import archive

        episode = self.episode(archived=False)
        path = self.path_for(episode)
        write_id3(path, [("a", 0, 5000, "Un")])
        rules.save({"chapters": False})
        try:
            archive._finalize_episode(episode, path)
        finally:
            rules.save({"chapters": True})
        self.assertFalse(PodcastChapterScan.select().exists())
        self.assertTrue(os.path.isfile(episode.path or path))

    def test_the_command_scans_once_then_only_with_force(self):
        from click.testing import CliRunner

        from neezer.cli import cli

        episode = self.episode(archived=True)
        write_id3(episode.path, [("a", 0, 5000, "Un")])
        runner = CliRunner()
        first = runner.invoke(cli, ["podcast", "chapters"])
        self.assertEqual(first.exit_code, 0, first.output)
        self.assertIn("scanned=1", first.output)
        again = runner.invoke(cli, ["podcast", "chapters"])
        self.assertIn("scanned=0", again.output)
        forced = runner.invoke(cli, ["podcast", "chapters", "--force"])
        self.assertIn("scanned=1", forced.output)
        self.assertEqual(PodcastChapter.select().count(), 1)


def count_statements():
    counts = collections.Counter()
    original = peewee.Database.execute_sql

    def execute_sql(self, sql, *args, **kwargs):
        counts[sql.split(None, 1)[0].upper()] += 1
        return original(self, sql, *args, **kwargs)

    @contextlib.contextmanager
    def run():
        peewee.Database.execute_sql = execute_sql
        try:
            yield counts
        finally:
            peewee.Database.execute_sql = original

    return run()


if __name__ == "__main__":
    unittest.main()
