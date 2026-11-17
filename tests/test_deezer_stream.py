# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""The audio path: from Deezer's encrypted bytes to an archive that plays forever.

Every Deezer track anybody hears goes through three functions that no test used
to run: ``provider.blowfish_key``, ``provider.iter_decrypted`` (every third
2048-byte chunk of the stream is Blowfish-CBC) and ``archive.open_live_stream``,
the tee that plays a cold track while it downloads and keeps it. The tests that
archived a track replaced the download with a function writing 4096 zero bytes —
which the archive then accepted as a FLAC, failed to tag, and kept for good.

So these run the real code over a real HTTP connection, against a local stand-in
for the CDN that delivers a body the way a network does: in pieces that line up
with nothing, sometimes cut short. What they check is what a listener gets (the
original audio, byte for byte, as it arrives) and what the archive keeps (that
audio, tagged, or nothing at all).
"""

import hashlib
import http.server
import os
import random
import shlex
import shutil
import tempfile
import threading
import time
import types
import unittest
from pathlib import Path
from unittest import mock

import mutagen.flac
import mutagen.id3
import requests
from Crypto.Cipher import Blowfish

from deezerpy import new_session
from deezerpy._circuit import breaker

from neezer.db import Track
from neezer.deezer import archive, ids, library
from neezer.deezer import provider as provider_mod
from neezer.deezer.provider import DeezerError, DeezerProvider, blowfish_key

from .testbase import TestBase

ASSETS = Path(__file__).parent / "assets"
FLAC = (ASSETS / "formats" / "silence.flac").read_bytes()
MP3 = (ASSETS / "formats" / "silence.mp3").read_bytes()
COVER = (ASSETS / "cover.jpg").read_bytes()

# Deezer's scheme, pinned by implementations that share no code with ours. The
# keys come from Node's md5 (key[i] = md5hex[i] ^ md5hex[i + 16] ^ secret[i]);
# the digest from OpenSSL's Blowfish:
#   openssl enc -e -bf-cbc -nopad -K <key of 3135556> -iv 0001020304050607 \
#       -provider legacy -provider default
KEYS = {
    "3135556": "6c6c666b39662c37652575603c643439",
    "1": "3464656e343a7d3a672c236a33696061",
    "916424": "61336f6a33692660317e2764696a3166",
}
IV = b"\x00\x01\x02\x03\x04\x05\x06\x07"
BLOCK = bytes((i * 7 + 3) % 256 for i in range(2048))
BLOCK_ENCRYPTED_SHA256 = "48ceb46f82f435de61a879f2f772c5eff0f380f0bc7399f685ede31f2b9e23c0"


def encrypt_like_deezer(plain: bytes, track_id) -> bytes:
    """What the CDN serves for a track: the 2048-byte blocks at offsets 0, 6144,
    12288... encrypted on their own (the IV starts over each time), everything
    else — including a short last block — in the clear."""
    key = bytes.fromhex(KEYS[str(track_id)])
    out = bytearray()
    for offset in range(0, len(plain), 2048):
        block = plain[offset : offset + 2048]
        if offset % 6144 == 0 and len(block) == 2048:
            block = Blowfish.new(key, Blowfish.MODE_CBC, IV).encrypt(block)
        out += block
    return bytes(out)


def fake_flac(size: int, seed: int = 0) -> bytes:
    """Bytes that open like a FLAC, for sizes the real asset does not have."""
    return b"fLaC" + random.Random(seed).randbytes(size - 4)


def wait_until(predicate, timeout=15.0, step=0.02) -> bool:
    """Bounded poll of an observable state, the way a client waits for it."""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return True
        time.sleep(step)
    return bool(predicate())


def live_threads() -> list:
    return [t for t in threading.enumerate() if t.name == "live-archive" and t.is_alive()]


def _need_ffmpeg() -> None:
    if shutil.which("ffmpeg"):
        return
    if os.environ.get("NS_REQUIRE_FFMPEG") == "1":
        raise RuntimeError("NS_REQUIRE_FFMPEG=1 but ffmpeg is not on PATH")
    raise unittest.SkipTest("ffmpeg not installed")


def write_flac(path, channels, seconds=10) -> Path:
    """Pink noise as a real FLAC, ``channels`` wide.

    Two INDEPENDENT channels when stereo, not one signal copied into both: a
    dual-mono source lets libopus halve its own rate (measured 263 kbit/s for a
    320 kbit/s target), so it can neither show what a stereo track really costs
    nor what a mono one does. The 15 KB of silence above is smaller than the
    32 KB ffmpeg reads before it emits anything, so it could not show a stream
    that starts before its download ends either.
    """
    import subprocess

    inputs, labels = [], []
    for i in range(channels):
        inputs += ["-f", "lavfi", "-i",
                   f"anoisesrc=d={seconds}:c=pink:a=0.3:seed={7 + 4 * i}"]
        labels.append(f"[{i}:a]")
    cmd = ["ffmpeg", "-v", "0", *inputs]
    if channels > 1:
        cmd += ["-filter_complex", "".join(labels) + f"amerge=inputs={channels}"]
    cmd += ["-ac", str(channels), "-ar", "44100", "-y", str(path)]
    subprocess.run(cmd, check=True)
    return Path(path)


def music_flac(directory) -> bytes:
    """Ten seconds of stereo pink noise as FLAC: ~2 MB, a real track's bitrate."""
    path = write_flac(Path(directory) / "music.flac", 2)
    data = path.read_bytes()
    path.unlink()
    return data


def mono_flac(directory) -> bytes:
    """The same, one channel: a mono record, which is what libopus refuses 320k of."""
    path = write_flac(Path(directory) / "mono.flac", 1)
    data = path.read_bytes()
    path.unlink()
    return data


def decoded_seconds(body: bytes) -> float:
    """How much audio a body carries, by decoding it (no header trust)."""
    import subprocess

    with tempfile.NamedTemporaryFile(suffix=".bin", delete=False) as fp:
        fp.write(body)
        path = fp.name
    try:
        out = subprocess.run(
            ["ffmpeg", "-v", "info", "-i", path, "-f", "null", "-"],
            capture_output=True, text=True,
        ).stderr
    finally:
        os.remove(path)
    total = 0.0
    for line in out.replace("\r", "\n").splitlines():
        marker = line.find("time=")
        if marker < 0:
            continue
        hours, minutes, seconds = line[marker + 5 :].split(" ")[0].split(":")
        total = max(total, int(hours) * 3600 + int(minutes) * 60 + float(seconds))
    return total


class Cdn:
    """A local stand-in for Deezer's CDN.

    Bodies are written in pieces that are never a multiple of 2048, with a
    pause every few so the client really does receive them apart; ``chunked``
    also frames them as HTTP chunks of those sizes. ``*-truncated`` sends two
    thirds and hangs up. ``gated`` sends three blocks (or ``head`` bytes), then
    waits for ``gate`` before the rest, and records when it did.
    """

    PIECES = (1, 7, 1000, 2047, 3, 2049, 4096, 13, 3001, 511)

    def __init__(self):
        self.bodies = {}
        self.hits = []
        self.gate = threading.Event()
        self.rest_sent = threading.Event()
        cdn = self

        class Handler(http.server.BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def log_message(self, *args):
                pass

            def handle(self):
                try:
                    super().handle()
                except (BrokenPipeError, ConnectionResetError):
                    pass  # the listener left, which is what some tests are about

            def do_GET(self):
                cdn.hits.append(self.path)
                body, mode, head = cdn.bodies.get(self.path, (None, None, 6144))
                if body is None or mode == "forbidden":
                    self.send_response(404 if body is None else 403)
                    self.send_header("Content-Length", "0")
                    self.end_headers()
                    return
                self._send(body, mode, head)

            def _send(self, body, mode, head_size=6144):
                chunked = mode.startswith("chunked")
                truncated = mode.endswith("truncated")
                self.send_response(200)
                self.send_header("Content-Type", "application/octet-stream")
                if chunked:
                    self.send_header("Transfer-Encoding", "chunked")
                else:
                    self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                if truncated:
                    body = body[: len(body) * 2 // 3]
                    self.close_connection = True
                head, rest = (body[:head_size], body[head_size:]) if mode == "gated" else (body, b"")
                self._pieces(head, chunked)
                if mode == "gated":
                    cdn.gate.wait(5)
                    cdn.rest_sent.set()
                    self._pieces(rest, chunked)
                if chunked and not truncated:
                    self.wfile.write(b"0\r\n\r\n")

            def _pieces(self, data, chunked):
                offset = n = 0
                while offset < len(data):
                    piece = data[offset : offset + Cdn.PIECES[n % len(Cdn.PIECES)]]
                    offset += len(piece)
                    self.wfile.write(b"%x\r\n%s\r\n" % (len(piece), piece) if chunked else piece)
                    self.wfile.flush()
                    n += 1
                    if n % 4 == 0:
                        time.sleep(0.001)

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.server.daemon_threads = True
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.base = f"http://127.0.0.1:{self.server.server_address[1]}"

    def serve(self, name: str, body: bytes, mode: str = "length", head: int = 6144) -> str:
        path = "/" + name.replace(" ", "-")
        self.bodies[path] = (body, mode, head)
        return self.base + path

    def close(self):
        self.server.shutdown()
        self.server.server_close()


class FakeGW:
    """Deezer is no lyrics source any more: an archive must never ask it for any."""

    def __init__(self):
        self.lyrics_calls = []

    LYRICS = {
        "LYRICS_TEXT": "Work it harder\nMake it better",
        "LYRICS_SYNC_JSON": [
            {"line": "Work it harder", "milliseconds": "1500"},
            {"line": "Make it better", "milliseconds": "3250"},
        ],
    }

    def get_track_lyrics(self, sng_id):
        self.lyrics_calls.append(sng_id)
        return self.LYRICS


def gw_info(sid, **extra):
    """A ``song.getData`` payload, as rich as the one an archive is finalized from."""
    info = {
        "SNG_ID": str(sid),
        "SNG_TITLE": "Harder",
        "VERSION": "(Live)",
        "ART_ID": "27",
        "ART_NAME": "Daft Punk",
        "ALB_ID": "302127",
        "ALB_TITLE": "Alive",
        "ALB_PICTURE": "coverhash",
        "DURATION": "5",
        "TRACK_NUMBER": "4",
        "DISK_NUMBER": "1",
        "PHYSICAL_RELEASE_DATE": "2007-11-19",
        "ISRC": "GBDUW0000059",
        "GAIN": "-8.4",
        "ARTISTS": [
            {"ART_ID": "27", "ART_NAME": "Daft Punk", "ARTISTS_SONGS_ORDER": "0"},
            {"ART_ID": "99", "ART_NAME": "Guest", "ARTISTS_SONGS_ORDER": "1"},
        ],
    }
    info.update(extra)
    return info


class _CdnMixin:
    @classmethod
    def setUpClass(cls):
        super().setUpClass()
        cls.cdn = Cdn()
        # The session production uses (timeouts, circuit breaker), minus the
        # environment's proxy settings: the CDN here is on loopback.
        cls.session = new_session()
        cls.session.trust_env = False

    @classmethod
    def tearDownClass(cls):
        cls.session.close()
        cls.cdn.close()
        super().tearDownClass()

    def _provider(self, archive_dir):
        breaker.reset()
        provider = DeezerProvider("dummy-arl", archive_dir, "FLAC")
        provider._dz = types.SimpleNamespace(
            session=self.session, http_headers={"User-Agent": "tests"}, gw=FakeGW()
        )
        return provider

    def _files(self, directory):
        return sorted(
            os.path.relpath(os.path.join(d, f), directory)
            for d, _dirs, files in os.walk(directory)
            for f in files
        )


class KnownAnswerTestCase(unittest.TestCase):
    def test_the_key_is_deezers(self):
        for track_id, key in KEYS.items():
            self.assertEqual(blowfish_key(track_id).hex(), key, track_id)
        # The gateway hands ids over as strings or numbers; the key is the id's.
        self.assertEqual(blowfish_key(3135556), blowfish_key("3135556"))

    def test_the_streams_these_tests_build_are_the_ones_the_cdn_serves(self):
        # The encryption below is what every other test feeds the real code, so
        # it is checked against OpenSSL rather than trusted.
        encrypted = Blowfish.new(bytes.fromhex(KEYS["3135556"]), Blowfish.MODE_CBC, IV).encrypt(BLOCK)
        self.assertEqual(hashlib.sha256(encrypted).hexdigest(), BLOCK_ENCRYPTED_SHA256)
        stream = encrypt_like_deezer(BLOCK * 7, "3135556")
        for index in range(7):
            block = stream[index * 2048 : (index + 1) * 2048]
            self.assertEqual(block == encrypted, index % 3 == 0, index)


class BlocksTestCase(unittest.TestCase):
    """``decrypt_blocks`` on its own: any cut of the body, the same file."""

    def test_the_file_comes_back_whatever_pieces_it_arrives_in(self):
        rng = random.Random(7)
        key = bytes.fromhex(KEYS["3135556"])
        for size in (2048, 3 * 2048 + 5, 7 * 2048, 7 * 2048 + 1999):
            plain = fake_flac(size, seed=size)
            body = encrypt_like_deezer(plain, "3135556")
            for trial in range(100):
                cuts = sorted(rng.sample(range(1, size), rng.randint(0, min(40, size - 1))))
                pieces = [body[a:b] for a, b in zip([0] + cuts, cuts + [size])]
                with self.subTest(size=size, trial=trial):
                    out = list(provider_mod.decrypt_blocks(pieces, key))
                    self.assertEqual(b"".join(out), plain)
                    # Whole blocks until the tail: the first thing handed on is
                    # always the first block, which is what gets checked for audio.
                    self.assertTrue(all(len(o) % 2048 == 0 for o in out[:-1]))
                    self.assertGreaterEqual(len(out[0]), min(size, 2048))

    def test_what_opens_like_audio(self):
        for head in (FLAC[:8], MP3[:8], b"\xff\xfb\x90\x64", b"\xff\xf3\x00", b"ID3\x03"):
            self.assertTrue(provider_mod.looks_like_audio(head), head)
        for head in (b"", b"f", b"<html>", b"\x00\x00\x00\x00", b"\xff\x1f", b"fLa", b"RIFF"):
            self.assertFalse(provider_mod.looks_like_audio(head), head)


class DecryptionTestCase(_CdnMixin, unittest.TestCase):
    """``iter_decrypted`` against the real network stack."""

    def setUp(self):
        self.dir = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.dir, ignore_errors=True)
        self.provider = self._provider(self.dir)

    def _decrypt(self, url, track_id="3135556"):
        return b"".join(self.provider.iter_decrypted(url, track_id))

    def test_it_undoes_the_cdns_encryption_however_the_bytes_arrive(self):
        cases = {
            "the real FLAC": FLAC,
            "one block": fake_flac(2048),
            # Blocks 0, 3 and 6 are encrypted: each starts its own CBC chain.
            "seven blocks": fake_flac(7 * 2048, seed=1),
            # A short block where an encrypted one would be is left alone —
            # 5 bytes cannot even be decrypted, 16 bytes can and must not be.
            "short tail of 5": fake_flac(3 * 2048 + 5, seed=2),
            "short tail of 16": fake_flac(3 * 2048 + 16, seed=3),
        }
        for name, plain in cases.items():
            for mode in ("length", "chunked"):
                with self.subTest(name, mode=mode):
                    url = self.cdn.serve(f"{name}-{mode}", encrypt_like_deezer(plain, "3135556"), mode)
                    self.assertEqual(self._decrypt(url), plain)

    def test_it_hands_audio_on_as_it_arrives(self):
        """The whole point of the live stream: the first second plays while the
        rest is still on its way."""
        self.cdn.gate.clear()
        self.cdn.rest_sent.clear()
        plain = fake_flac(10 * 2048, seed=4)
        url = self.cdn.serve("gated", encrypt_like_deezer(plain, "3135556"), "gated")
        chunks = self.provider.iter_decrypted(url, "3135556")
        first = next(chunks)
        self.assertFalse(self.cdn.rest_sent.is_set(), "the first chunk waited for the whole body")
        self.assertEqual(first, plain[:2048])
        self.cdn.gate.set()
        self.assertEqual(first + b"".join(chunks), plain)

    def test_a_body_cut_short_is_an_error_not_a_shorter_track(self):
        plain = fake_flac(6 * 2048, seed=5)
        for mode in ("length-truncated", "chunked-truncated"):
            with self.subTest(mode):
                url = self.cdn.serve(f"cut-{mode}", encrypt_like_deezer(plain, "3135556"), mode)
                with self.assertRaises(requests.RequestException):
                    self._decrypt(url)

    def test_a_refusal_from_the_cdn_is_an_error(self):
        url = self.cdn.serve("expired", b"x", "forbidden")
        with self.assertRaises(requests.HTTPError):
            self._decrypt(url)

    def test_what_does_not_decrypt_to_audio_is_refused_before_a_byte_is_handed_on(self):
        plain = fake_flac(4 * 2048, seed=6)
        cases = {
            # The key of another track: what a wrong id or a changed scheme yields.
            "wrong key": encrypt_like_deezer(plain, "1"),
            # An error page answered with a 200, short enough to arrive in clear...
            "error page": b"<html><body>403 Forbidden</body></html>",
            # ...or long enough to be "decrypted" into noise.
            "long error page": b"<html>" + b" " * 3000 + b"</html>",
        }
        for name, body in cases.items():
            with self.subTest(name):
                url = self.cdn.serve(f"bad-{name}", body)
                received = []
                with self.assertRaises(DeezerError):
                    for chunk in self.provider.iter_decrypted(url, "3135556"):
                        received.append(chunk)
                self.assertEqual(received, [])

    def test_an_empty_body_is_not_a_track(self):
        url = self.cdn.serve("empty", b"")
        with self.assertRaises(DeezerError):
            self._decrypt(url)


class DownloadToTestCase(_CdnMixin, unittest.TestCase):
    """The background path: ``download_to`` publishes a whole file or nothing."""

    def setUp(self):
        self.dir = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.dir, ignore_errors=True)
        self.provider = self._provider(self.dir)
        self.dest = Path(self.dir, "Artist", "Album", "01 Track.flac")

    def test_it_publishes_the_decrypted_file(self):
        url = self.cdn.serve("dl-ok", encrypt_like_deezer(FLAC, "3135556"), "chunked")
        self.provider.download_to(url, "3135556", self.dest)
        self.assertEqual(self.dest.read_bytes(), FLAC)
        self.assertEqual(self._files(self.dir), [os.path.join("Artist", "Album", "01 Track.flac")])

    def test_a_failed_download_leaves_nothing_behind(self):
        # Nothing else ever removes a stray partial: it would sit in the archive
        # (and in its reported size) for good.
        bad = {
            "cut": (encrypt_like_deezer(FLAC, "3135556"), "length-truncated"),
            "garbage": (encrypt_like_deezer(FLAC, "1"), "length"),
            "empty": (b"", "length"),
            "refused": (b"x", "forbidden"),
        }
        for name, (body, mode) in bad.items():
            with self.subTest(name):
                url = self.cdn.serve(f"dl-{name}", body, mode)
                with self.assertRaises(Exception):
                    self.provider.download_to(url, "3135556", self.dest)
                self.assertEqual(self._files(self.dir), [])

    def test_a_failed_download_keeps_the_previous_archive(self):
        url = self.cdn.serve("dl-first", encrypt_like_deezer(FLAC, "3135556"))
        self.provider.download_to(url, "3135556", self.dest)
        url = self.cdn.serve("dl-cut", encrypt_like_deezer(fake_flac(9000), "3135556"), "chunked-truncated")
        with self.assertRaises(requests.RequestException):
            self.provider.download_to(url, "3135556", self.dest)
        self.assertEqual(self.dest.read_bytes(), FLAC)


class LiveStreamTestCase(_CdnMixin, TestBase):
    """``open_live_stream``: a cold track, played while it downloads."""

    SID = "3135556"

    def setUp(self):
        super().setUp()
        self.archive_dir = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.archive_dir, ignore_errors=True)
        self.provider = self._provider(self.archive_dir)
        self.provider.fetch_cover = lambda md5, size=1000: COVER if md5 == "coverhash" else None
        self.analysed = []
        patcher = mock.patch(
            "neezer.deezer.analysis.queue_analysis",
            lambda track, provider=None: self.analysed.append(track.id),
        )
        patcher.start()
        self.addCleanup(patcher.stop)
        enrich = mock.patch("neezer.enrich.queue.queue_track", lambda *a, **k: None)
        enrich.start()
        self.addCleanup(enrich.stop)
        root = library.get_root_folder(self.archive_dir)
        self.track = library.upsert_track(gw_info(self.SID), root, "FLAC")
        self.aborted = 0

    def _on_abort(self):
        self.aborted += 1

    def _resolve_to(self, url, fmt="FLAC", used_id=None, info=None):
        answer = (url, fmt, info or gw_info(self.SID), used_id or self.SID)
        self.provider.resolve = lambda sng_id, quality=None: answer

    def _open(self):
        return archive.open_live_stream(self.provider, self.track, self._on_abort)

    def _leftovers(self):
        return [f for f in self._files(self.archive_dir) if ".part" in f]

    def _part_bytes(self) -> int:
        total = 0
        for rel in self._leftovers():
            try:
                total += os.path.getsize(os.path.join(self.archive_dir, rel))
            except OSError:
                pass
        return total

    def test_a_cold_track_plays_while_it_downloads_and_is_kept(self):
        self.cdn.gate.clear()
        self.cdn.rest_sent.clear()
        self._resolve_to(self.cdn.serve("live-ok", encrypt_like_deezer(FLAC, self.SID), "gated"))
        mimetype, stream = self._open()
        self.assertEqual(mimetype, "audio/flac")
        dest = Path(self.track.path)

        first = next(stream)
        self.assertFalse(self.cdn.rest_sent.is_set(), "the listener waited for the whole file")
        self.assertFalse(dest.exists(), "a partial download was published as the archive")
        self.cdn.gate.set()
        heard = first + b"".join(stream)

        # The listener heard the original audio, exactly.
        self.assertEqual(heard, FLAC)
        # And the archive holds that audio, tagged from the gateway payload.
        audio = mutagen.flac.FLAC(dest)
        original = mutagen.flac.FLAC(ASSETS / "formats" / "silence.flac")
        self.assertEqual(audio.info.md5_signature, original.info.md5_signature)
        self.assertEqual(audio.info.total_samples, original.info.total_samples)
        self.assertEqual(audio["title"], ["Harder (Live)"])
        self.assertEqual(audio["artist"], ["Daft Punk", "Guest"])
        self.assertEqual(audio["album"], ["Alive"])
        self.assertEqual(audio["tracknumber"], ["4"])
        self.assertEqual(audio["date"], ["2007-11-19"])
        self.assertEqual(audio["isrc"], ["GBDUW0000059"])
        # Deezer's GAIN is a loudness: the adjustment is -(GAIN + 18.4).
        self.assertNotIn("replaygain_track_gain", audio)  # the gain is measured, not Deezer's
        self.assertEqual([p.data for p in audio.pictures], [COVER])
        self.assertNotIn("lyrics", audio)  # lyrics come from LRCLIB, lazily

        # Everything else about it is on disk too, beside the audio.
        folder = dest.parent
        self.assertEqual((folder / "cover.jpg").read_bytes(), COVER)
        self.assertFalse(dest.with_suffix(".lrc").exists())
        self.assertTrue(os.path.isfile(library.metadata_path(self.track)))
        self.assertEqual(self._leftovers(), [])

        # The row says so, and the analysis was queued for it.
        row = Track[ids.track_uuid(self.SID)]
        self.assertEqual(row.path, str(dest))
        self.assertGreater(row.last_modification, 0)
        self.assertTrue(row.has_art)
        self.assertIsNone(row.unavailable)
        self.assertEqual(row.bitrate, int(os.path.getsize(dest) * 8 / 5 / 1000))
        self.assertEqual(self.analysed, [row.id])
        self.assertEqual(self.aborted, 0)

    def test_a_fallback_is_decrypted_with_the_key_of_the_track_actually_served(self):
        # Deezer substitutes another version (FALLBACK.SNG_ID) for a track it
        # cannot serve; the stream is encrypted for THAT id.
        self._resolve_to(
            self.cdn.serve("live-fallback", encrypt_like_deezer(FLAC, "916424")), used_id="916424"
        )
        _mimetype, stream = self._open()
        self.assertEqual(b"".join(stream), FLAC)
        self.assertEqual(mutagen.flac.FLAC(self.track.path).info.total_samples, 214272)

    def test_an_mp3_when_there_is_no_flac(self):
        self._resolve_to(self.cdn.serve("live-mp3", encrypt_like_deezer(MP3, self.SID)), fmt="MP3_320")
        mimetype, stream = self._open()
        self.assertEqual(mimetype, "audio/mpeg")
        self.assertEqual(b"".join(stream), MP3)
        self.assertTrue(self.track.path.endswith(".mp3"))
        tags = mutagen.id3.ID3(self.track.path)
        self.assertEqual(tags["TIT2"].text, ["Harder (Live)"])
        self.assertEqual(tags["TPE1"].text, ["Daft Punk", "Guest"])
        self.assertEqual(tags.getall("APIC")[0].data, COVER)
        self.assertEqual(tags.getall("USLT"), [])
        self.assertEqual(Track[ids.track_uuid(self.SID)].path, self.track.path)

    def test_a_listener_who_leaves_does_not_stop_the_download(self):
        self._resolve_to(self.cdn.serve("live-leave", encrypt_like_deezer(FLAC, self.SID), "chunked"))
        _mimetype, stream = self._open()
        next(stream)
        stream.close()  # what the WSGI server does when the client hangs up
        self.assertTrue(
            wait_until(lambda: os.path.exists(self.track.path)),
            "the archive waited for a listener that had gone",
        )
        audio = mutagen.flac.FLAC(self.track.path)
        original = mutagen.flac.FLAC(ASSETS / "formats" / "silence.flac")
        self.assertEqual(audio.info.md5_signature, original.info.md5_signature)
        self.assertTrue(wait_until(lambda: not self._leftovers(), timeout=5))
        self.assertTrue(
            wait_until(lambda: Track[ids.track_uuid(self.SID)].last_modification > 0)
        )
        self.assertEqual(self.aborted, 0)
        self.assertEqual(self.analysed, [Track[ids.track_uuid(self.SID)].id])

    @unittest.skipUnless(shutil.which("ffmpeg"), "needs ffmpeg")
    def test_a_client_that_stops_reading_does_not_pace_the_download(self):
        """The reported bug: the browser buffers enough and stops reading, and the
        fetch stopped with it — the archive stayed a .part for minutes."""
        music = self._music_flac()
        self.cdn.gate.clear()
        self.cdn.rest_sent.clear()
        self._resolve_to(
            self.cdn.serve(
                "live-stall", encrypt_like_deezer(music, self.SID), "gated", head=256 * 1024
            )
        )
        _mimetype, stream = self._open()
        first = next(stream)
        self.assertTrue(first)
        self.cdn.gate.set()
        self.assertFalse(self.cdn.rest_sent.is_set())
        # The listener reads NOTHING more from here on (a full buffer, a paused
        # tab): the archive must still complete on its own.
        self.assertTrue(
            wait_until(lambda: os.path.exists(self.track.path), timeout=25),
            "the download was paced by the listener",
        )
        self.assertTrue(
            wait_until(lambda: Track[ids.track_uuid(self.SID)].last_modification > 0)
        )
        audio = mutagen.flac.FLAC(self.track.path)
        self.assertEqual(audio.info.total_samples, 441000)
        self.assertEqual(audio["title"], ["Harder (Live)"])
        self.assertEqual(self.cdn.hits.count("/live-stall"), 1)
        self.assertEqual(self.aborted, 0)
        # The .part is the stalled listener's copy: it goes when that listener does.
        stream.close()
        self.assertTrue(wait_until(lambda: not self._leftovers(), timeout=5))
        self.assertTrue(wait_until(lambda: not live_threads(), timeout=5))

    @unittest.skipUnless(shutil.which("ffmpeg"), "needs ffmpeg")
    def test_a_reload_during_the_grace_reuses_the_same_download(self):
        music = self._music_flac()
        opened = []
        resolved = []

        def slow(url, tid):
            opened.append(url)
            step = max(1, len(music) // 40)
            for offset in range(0, len(music), step):
                yield music[offset : offset + step]
                time.sleep(0.03)

        self.provider.iter_decrypted = slow
        self.provider.resolve = lambda sng_id, quality=None: (
            resolved.append(sng_id) or ("http://127.0.0.1:1/never-read", "FLAC", gw_info(self.SID), self.SID)
        )
        _mimetype, first = self._open()
        self.assertTrue(next(first))
        first.close()  # the chase reload drops the response...
        _mimetype, second = self._open()  # ...and asks for the same track again
        self.assertEqual(b"".join(second), music)
        self.assertEqual(len(opened), 1, "the reload started a second download")
        self.assertEqual(len(resolved), 1, "the reload asked Deezer for the stream again")
        self.assertTrue(wait_until(lambda: os.path.exists(self.track.path)))
        self.assertTrue(wait_until(lambda: not live_threads(), timeout=5))
        self.assertEqual(self.aborted, 0)

    @unittest.skipUnless(shutil.which("ffmpeg"), "needs ffmpeg")
    def test_an_abandoned_download_stops_after_the_grace_and_leaves_nothing(self):
        music = self._music_flac()

        def slow(url, tid):
            step = max(1, len(music) // 60)
            for offset in range(0, len(music), step):
                yield music[offset : offset + step]
                time.sleep(0.05)

        self.provider.iter_decrypted = slow
        self._resolve_to("http://127.0.0.1:1/never-read")
        _mimetype, stream = self._open()
        next(stream)
        with mock.patch.object(archive, "LIVE_GRACE_SECONDS", 0.4):
            stream.close()  # the person skipped to something else
            held = self._part_bytes()
            self.assertTrue(
                wait_until(lambda: self._part_bytes() > held, timeout=5),
                "the download stopped with the listener instead of finishing the grace",
            )
            self.assertTrue(
                wait_until(lambda: not self._leftovers(), timeout=20),
                "the orphaned download never stopped",
            )
            self.assertTrue(wait_until(lambda: not live_threads(), timeout=5))
        self.assertEqual(self.aborted, 1)
        self.assertFalse(os.path.exists(self.track.path))
        self.assertEqual(Track[ids.track_uuid(self.SID)].last_modification, 0)

    def test_an_abandoned_download_yields_the_link_to_the_next_track(self):
        chunk = b"x" * 65536
        served = []

        def fast_source(url, tid):
            while len(served) < 100:
                served.append(1)
                yield chunk
                time.sleep(0.01)

        self.provider.iter_decrypted = fast_source
        self._resolve_to("http://127.0.0.1:1/never-read")
        _mimetype, stream = self._open()
        next(stream)
        with mock.patch.object(archive, "LIVE_GRACE_SECONDS", 0.9), mock.patch.object(
            archive, "LIVE_REUSE_SECONDS", 0.2
        ), mock.patch.object(archive, "LIVE_ORPHAN_PACE", 0.3):
            stream.close()
            self.assertTrue(wait_until(lambda: not live_threads(), timeout=20))
        self.assertLess(
            len(served),
            40,
            "an abandoned download kept taking the link at full speed",
        )
        self.assertGreaterEqual(len(served), 2, "it stopped before its grace was over")
        self.assertEqual(self.aborted, 1)

    def test_a_file_that_already_exists_is_served_without_a_download(self):
        Path(self.track.path).parent.mkdir(parents=True, exist_ok=True)
        Path(self.track.path).write_bytes(FLAC)
        self.provider.resolve = lambda *a, **k: (_ for _ in ()).throw(
            AssertionError("a file on disk needed no Deezer call")
        )
        _mimetype, stream = self._open()
        self.assertEqual(b"".join(stream), FLAC)
        self.assertEqual(self.cdn.hits.count("/live-present"), 0)
        self.assertFalse(live_threads())

    @unittest.skipUnless(shutil.which("ffmpeg"), "needs ffmpeg")
    def test_a_slow_reader_hears_the_audio_unchanged_while_it_is_published(self):
        """The archive is a tagged COPY: a reader still on the raw file must hear
        exactly what was downloaded, not a file being retagged under it."""
        music = self._music_flac()
        self._resolve_to(self.cdn.serve("live-slow", encrypt_like_deezer(music, self.SID), "chunked"))
        _mimetype, stream = self._open()
        heard = bytearray(next(stream))
        self.assertTrue(wait_until(lambda: os.path.exists(self.track.path), timeout=15))
        for chunk in stream:
            heard += chunk
        self.assertEqual(bytes(heard), music)
        self.assertEqual(mutagen.flac.FLAC(self.track.path).info.total_samples, 441000)
        self.assertTrue(wait_until(lambda: not live_threads(), timeout=5))

    @unittest.skipUnless(shutil.which("ffmpeg"), "needs ffmpeg")
    def test_a_download_that_has_decided_to_stop_is_not_joined(self):
        music = self._music_flac()

        def slow(url, tid):
            step = max(1, len(music) // 60)
            for offset in range(0, len(music), step):
                yield music[offset : offset + step]
                time.sleep(0.05)

        self.provider.iter_decrypted = slow
        self._resolve_to("http://127.0.0.1:1/never-read")
        _mimetype, stream = self._open()
        next(stream)
        live = archive._live_downloads[archive._live_key(self.provider, self.track)]
        with live._cond:
            live._stopping = True
        try:
            self.assertFalse(live.running)
            self.assertIsNone(archive._live_lookup(live._key))
        finally:
            with mock.patch.object(archive, "LIVE_GRACE_SECONDS", 0.2):
                stream.close()
                self.assertTrue(wait_until(lambda: not live_threads(), timeout=20))

    @unittest.skipUnless(shutil.which("ffmpeg"), "needs ffmpeg")
    def test_a_stalled_source_fails_the_response_instead_of_hanging_it(self):
        music = self._music_flac()

        def stuck(url, tid):
            yield music[:65536]
            time.sleep(5)

        self.provider.iter_decrypted = stuck
        self._resolve_to("http://127.0.0.1:1/never-read")
        _mimetype, stream = self._open()
        self.assertTrue(next(stream))
        with mock.patch.object(archive, "LIVE_STALL_SECONDS", 0.2), mock.patch.object(
            archive, "LIVE_GRACE_SECONDS", 0.2
        ):
            started = time.monotonic()
            with self.assertRaises(DeezerError):
                next(stream)
            self.assertLess(time.monotonic() - started, 3)
            stream.close()
            self.assertTrue(wait_until(lambda: not live_threads(), timeout=10))

    # -- the same live download, transcoded to Opus as it arrives -------------
    # The Opus path used to archive the WHOLE FLAC before its first byte went
    # out; measured at 512 KB/s on a minute of FLAC at a real music bitrate,
    # 13 s of silence against 0.77 s now.

    def _ffmpeg(self):
        _need_ffmpeg()

    def _music_flac(self):
        """Ten seconds of stereo pink noise as FLAC: ~1.6 MB, a real track's
        bitrate. The 15 KB of silence above is smaller than the 32 KB ffmpeg
        reads before it emits anything, so it could not show a stream that
        starts before its download ends."""
        return music_flac(self.archive_dir)

    def _opus(self, bitrate=64):
        from neezer.webui import _opus_live_generator

        aborted = threading.Event()
        self._on_abort = lambda: (setattr(self, "aborted", self.aborted + 1), aborted.set())
        _mimetype, stream = self._open()
        first = next(stream)
        return _opus_live_generator(first, stream, bitrate), aborted

    def test_opus_starts_while_the_flac_is_still_downloading_and_the_flac_is_kept(self):
        import mutagen.oggopus

        self._ffmpeg()
        music = self._music_flac()
        self.cdn.gate.clear()
        self.cdn.rest_sent.clear()
        # The CDN sends the first 256 KB (1.6 s of this audio) and holds the rest.
        self._resolve_to(
            self.cdn.serve("live-opus", encrypt_like_deezer(music, self.SID), "gated", head=256 * 1024)
        )
        opus, _aborted = self._opus()
        got = []
        reader = threading.Thread(target=lambda: got.append(next(opus)), daemon=True)
        reader.start()
        reader.join(4)  # the CDN holds the rest of the file back for 5 s
        self.assertTrue(got, "no Opus before the download finished")
        self.assertFalse(self.cdn.rest_sent.is_set(), "the listener waited for the whole file")
        self.assertTrue(got[0].startswith(b"OggS"))
        self.cdn.gate.set()
        heard = got[0] + b"".join(opus)

        # What the listener got is the whole track, in Opus...
        out = Path(self.archive_dir) / "heard.ogg"
        out.write_bytes(heard)
        self.assertAlmostEqual(mutagen.oggopus.OggOpus(out).info.length, 10.0, delta=0.05)
        # ...and the FLAC master was archived, tagged, exactly as it is played.
        audio = mutagen.flac.FLAC(self.track.path)
        self.assertEqual(audio.info.total_samples, 441000)
        self.assertEqual(audio["title"], ["Harder (Live)"])
        self.assertGreater(Track[ids.track_uuid(self.SID)].last_modification, 0)
        self.assertEqual(self._leftovers(), [])
        self.assertEqual(self.aborted, 0)

    def test_an_opus_listener_who_leaves_keeps_the_flac_archive(self):
        self._ffmpeg()
        self.cdn.gate.clear()
        # Held back mid-file: the listener leaves while it is still downloading.
        self._resolve_to(
            self.cdn.serve("live-opus-leave", encrypt_like_deezer(self._music_flac(), self.SID), "gated", head=256 * 1024)
        )
        opus, _aborted = self._opus()
        next(opus)
        opus.close()  # the client hangs up: ffmpeg is killed, its feeder stops
        self.cdn.gate.set()
        self.assertTrue(
            wait_until(lambda: os.path.exists(self.track.path)),
            "the FLAC download stopped with the Opus listener",
        )
        self.assertEqual(mutagen.flac.FLAC(self.track.path).info.total_samples, 441000)
        self.assertTrue(wait_until(lambda: not self._leftovers(), timeout=5))
        self.assertEqual(self.aborted, 0)
        self.assertGreater(Track[ids.track_uuid(self.SID)].last_modification, 0)

    def test_a_download_that_breaks_off_is_never_kept(self):
        self._resolve_to(self.cdn.serve("live-cut", encrypt_like_deezer(FLAC, self.SID), "length-truncated"))
        _mimetype, stream = self._open()
        with self.assertRaises(requests.RequestException):
            b"".join(stream)
        self.assertFalse(os.path.exists(self.track.path))
        self.assertEqual(self._leftovers(), [])
        self.assertEqual(self.aborted, 1)
        self.assertEqual(Track[ids.track_uuid(self.SID)].last_modification, 0)

    def test_a_stream_that_is_not_audio_is_neither_played_nor_kept(self):
        self._resolve_to(self.cdn.serve("live-garbage", encrypt_like_deezer(FLAC, "1")))
        _mimetype, stream = self._open()
        heard = []
        with self.assertRaises(DeezerError):
            for chunk in stream:
                heard.append(chunk)
        self.assertEqual(heard, [])
        self.assertFalse(os.path.exists(self.track.path))
        self.assertEqual(self._leftovers(), [])
        self.assertEqual(Track[ids.track_uuid(self.SID)].last_modification, 0)

    def test_a_track_archived_meanwhile_is_played_from_disk(self):
        url = self.cdn.serve("live-meanwhile", encrypt_like_deezer(FLAC, self.SID))
        self._resolve_to(url)
        _mimetype, stream = self._open()
        # Another worker finished it between the resolve and the first read.
        Path(self.track.path).parent.mkdir(parents=True, exist_ok=True)
        Path(self.track.path).write_bytes(FLAC)
        self.assertEqual(b"".join(stream), FLAC)
        self.assertNotIn("/live-meanwhile", self.cdn.hits)

    def test_when_another_download_publishes_first_the_stream_steps_aside(self):
        self._resolve_to(self.cdn.serve("live-race", encrypt_like_deezer(FLAC, self.SID), "chunked"))
        _mimetype, stream = self._open()
        heard = next(stream)
        # The background archiver publishes (and finalizes) while we play.
        other = b"fLaC" + b"the background archiver's copy"
        Path(self.track.path).write_bytes(other)
        heard += b"".join(stream)
        self.assertEqual(heard, FLAC)  # the listener still heard the whole track...
        self.assertEqual(Path(self.track.path).read_bytes(), other)  # ...the first archive stands...
        self.assertEqual(self._leftovers(), [])
        self.assertEqual(self.analysed, [])  # ...and is not finalized a second time
        self.assertEqual(self.aborted, 0)

    def test_an_archiver_that_finishes_while_the_stream_waits_for_the_lock_wins(self):
        """The background archiver holds the track's lock for its whole download.
        A stream that ends meanwhile has already looked for the file (not there
        yet) and waits; by the time it gets the lock the file is there, and it
        must look again rather than publish over it."""

        class HeldByTheArchiver:
            def __init__(self):
                self.lock = threading.Lock()
                self.waited_on = threading.Event()

            def __enter__(self):
                self.waited_on.set()
                self.lock.acquire()

            def __exit__(self, *exc):
                self.lock.release()

        held = HeldByTheArchiver()
        held.lock.acquire()
        self.provider._track_locks[self.SID] = held
        self._resolve_to(self.cdn.serve("live-lock", encrypt_like_deezer(FLAC, self.SID)))
        _mimetype, stream = self._open()
        heard = []
        listener = threading.Thread(target=lambda: heard.append(b"".join(stream)))
        listener.start()
        try:
            self.assertTrue(held.waited_on.wait(10), "the stream never tried to publish")
            other = b"fLaC" + b"the background archiver's copy"
            Path(self.track.path).write_bytes(other)
        finally:
            held.lock.release()
            listener.join(10)
        self.assertEqual(heard, [FLAC])
        self.assertEqual(Path(self.track.path).read_bytes(), other)
        self.assertEqual(self._leftovers(), [])
        self.assertEqual(self.analysed, [])

    def test_a_failure_while_finalizing_does_not_cut_the_music(self):
        self._resolve_to(self.cdn.serve("live-finalize", encrypt_like_deezer(FLAC, self.SID)))
        _mimetype, stream = self._open()
        with mock.patch.object(library, "refresh_track_metadata", side_effect=RuntimeError("db gone")), \
                self.assertLogs("neezer.deezer.archive", "WARNING"):
            self.assertEqual(b"".join(stream), FLAC)


class EpisodeFetchTestCase(_CdnMixin, unittest.TestCase):
    """Podcast episodes: plain audio from a third-party host, archived for good."""

    def setUp(self):
        self.dir = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.dir, ignore_errors=True)
        self.provider = self._provider(self.dir)
        # The SSRF guard refuses loopback by design (its own tests are in
        # test_deezer_resilience); here the host IS the loopback stand-in.
        patcher = mock.patch.object(provider_mod, "check_public_url", lambda url: ["127.0.0.1"])
        patcher.start()
        self.addCleanup(patcher.stop)

    def _fetch(self, url):
        received = []
        for chunk in self.provider.iter_episode(url):
            received.append(chunk)
        return b"".join(received)

    def test_whatever_audio_the_host_serves_arrives_as_it_was_sent(self):
        formats = {
            name: (ASSETS / "formats" / f"silence.{name}").read_bytes()
            for name in ("mp3", "m4a", "ogg", "flac")
        }
        # Something nobody listed as a format is still not a page.
        formats["unknown"] = b"\x00\x00\x01\xba" + bytes(range(256)) * 40
        for name, body in formats.items():
            for mode in ("length", "chunked"):
                with self.subTest(name, mode=mode):
                    self.assertEqual(self._fetch(self.cdn.serve(f"ep-{name}-{mode}", body, mode)), body)

    def test_a_page_instead_of_the_episode_is_refused(self):
        pages = {
            "html": b"<!DOCTYPE html><html><body>Episode removed</body></html>",
            "indented": b"\r\n  <html><head><title>Consent</title></head></html>",
            "bom": b"\xef\xbb\xbf<?xml version='1.0'?><error/>",
            "json": b'{"error": "not found"}',
            "empty": b"",
        }
        for name, body in pages.items():
            with self.subTest(name):
                received = []
                with self.assertRaises(DeezerError):
                    for chunk in self.provider.iter_episode(self.cdn.serve(f"page-{name}", body)):
                        received.append(chunk)
                self.assertEqual(received, [])

    def test_a_failed_episode_download_leaves_nothing_behind(self):
        dest = Path(self.dir, "Podcasts", "Show", "2026-09-26 Episode.mp3")
        for name, body, mode in (
            ("cut", MP3, "length-truncated"),
            ("page", b"<html>gone</html>", "length"),
            ("empty", b"", "length"),
        ):
            with self.subTest(name):
                with self.assertRaises(Exception):
                    self.provider.download_episode_to(self.cdn.serve(f"epdl-{name}", body, mode), dest)
                self.assertEqual(self._files(self.dir), [])
        self.provider.download_episode_to(self.cdn.serve("epdl-ok", MP3, "chunked"), dest)
        self.assertEqual(dest.read_bytes(), MP3)
        self.assertEqual(self._files(self.dir), [os.path.relpath(dest, self.dir)])


class OpusCeilingTestCase(unittest.TestCase):
    """libopus takes at most 256 kbit/s PER CHANNEL: ``OPUS_320`` must be clamped.

    On the image's static ffmpeg 9.0.2 (which ships no ffprobe), ``-b:a 320k`` on
    a MONO input exits 234 with *"The bit rate 320000 bps is unsupported. Please
    choose a value between 500 and 256000."* The very same request on a stereo
    input is accepted, and 512k is the stereo ceiling — so the guard cannot be a
    blanket 256 kbit/s: that would downgrade every stereo track.
    """

    def test_the_clamp_is_libopus_own_256_kbit_per_channel(self):
        from neezer.webui import _opus_kbps

        # Measured on ffmpeg 9.0.2: 320k accepted on 2 channels, refused on 1;
        # 512k accepted on 2, 513k refused.
        self.assertEqual(_opus_kbps(320, 1), 256)
        self.assertEqual(_opus_kbps(320, 2), 320)
        self.assertEqual(_opus_kbps(320, 6), 320)
        # A request the source can carry is left exactly as it is.
        self.assertEqual(_opus_kbps(128, 1), 128)
        self.assertEqual(_opus_kbps(64, 2), 64)
        self.assertEqual(_opus_kbps(192, 1), 192)

    def test_an_unknown_channel_count_forces_stereo_instead_of_guessing(self):
        from neezer.webui import _opus_args

        # A live source whose first bytes are not a FLAC (Deezer only has an MP3
        # when it has no FLAC): nothing to read the layout from, and a mono input
        # would be refused outright. A stereo output makes every rate we offer
        # valid and is a no-op on the stereo audio nearly all of it is.
        self.assertEqual(
            _opus_args(320, None),
            ["-c:a", "libopus", "-ac", "2", "-b:a", "320k", "-vbr", "on"],
        )
        # Known channels: no down/up-mix, just the ceiling.
        self.assertEqual(
            _opus_args(320, 2), ["-c:a", "libopus", "-b:a", "320k", "-vbr", "on"]
        )
        self.assertEqual(
            _opus_args(320, 1), ["-c:a", "libopus", "-b:a", "256k", "-vbr", "on"]
        )

    def test_a_flac_states_its_channel_count_in_its_first_bytes(self):
        """The live path has no file yet but already holds the first block.

        ``mediafile`` (mutagen) is the check for a finished file; the FLAC
        STREAMINFO parse is what makes the live path agree with it, and both are
        read here off real encoder output rather than a hand-built header.
        """
        from neezer.webui import _audio_channels, _flac_channels

        _need_ffmpeg()
        directory = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, directory, ignore_errors=True)
        for channels in (1, 2):
            with self.subTest(channels=channels):
                path = write_flac(Path(directory) / f"c{channels}.flac", channels)
                self.assertEqual(_audio_channels(str(path)), channels)
                head = path.read_bytes()[:65536]
                self.assertEqual(_flac_channels(head), channels)  # the live path's read
        # Anything that is not a FLAC (an MP3 fallback, a short/absent block)
        # answers None, and the caller then forces stereo instead of guessing.
        self.assertIsNone(_flac_channels(MP3))
        self.assertIsNone(_flac_channels(b""))
        self.assertIsNone(_flac_channels(None))
        self.assertIsNone(_flac_channels(b"fLaC"))


class StreamRouteTestCase(_CdnMixin, unittest.TestCase):
    """``/api/stream/<id>``, the web player's first play of a Deezer track."""

    SID = "3135556"

    def setUp(self):
        from neezer.config import DefaultConfig
        from neezer.db import release_database
        from neezer.deezer.workload import Priority
        from neezer.managers.user import UserManager
        from neezer.web import create_application

        fd, db_path = tempfile.mkstemp()
        os.close(fd)
        cache = tempfile.mkdtemp()
        self.archive_dir = tempfile.mkdtemp()
        self.addCleanup(os.remove, db_path)
        self.addCleanup(shutil.rmtree, cache, ignore_errors=True)
        self.addCleanup(shutil.rmtree, self.archive_dir, ignore_errors=True)

        class Config(DefaultConfig):
            TESTING = True

            def __init__(self):
                super().__init__()
                self.BASE = dict(self.BASE, database_uri="sqlite:///" + db_path)
                self.WEBAPP = dict(self.WEBAPP, cache_dir=cache, mount_webui=True, mount_api=True)

        self.app = create_application(Config())
        self.addCleanup(release_database)
        UserManager.add("alice", "Alic3", admin=True)
        self.provider = self._provider(self.archive_dir)
        self.provider.fetch_cover = lambda md5, size=1000: None
        self.app.deezer = self.provider
        self.queued = []
        self.app.deezer_prefetch = types.SimpleNamespace(
            download_ids=lambda ids, priority=None: self.queued.extend((i, priority) for i in ids)
        )
        self.app.config["DEEZER"]["archive_dir"] = self.archive_dir
        self.prefetch_priority = Priority.PREFETCH
        patcher = mock.patch("neezer.deezer.analysis.queue_analysis", lambda *a, **k: None)
        patcher.start()
        self.addCleanup(patcher.stop)
        enrich = mock.patch("neezer.enrich.queue.queue_track", lambda *a, **k: None)
        enrich.start()
        self.addCleanup(enrich.stop)

        root = library.get_root_folder(self.archive_dir)
        self.track = library.upsert_track(gw_info(self.SID), root, "FLAC")
        self.client = self.app.test_client()
        self.client.post("/api/login", json={"username": "alice", "password": "Alic3"})

    def _resolve_to(self, url):
        self.provider.resolve = lambda sng_id, quality=None: (url, "FLAC", gw_info(self.SID), self.SID)

    def test_a_first_play_streams_the_audio_and_the_next_one_is_served_from_disk(self):
        self._resolve_to(self.cdn.serve("route-ok", encrypt_like_deezer(FLAC, self.SID), "chunked"))
        rv = self.client.get(f"/api/stream/{self.SID}")
        self.assertEqual(rv.status_code, 200)
        self.assertEqual(rv.mimetype, "audio/flac")
        self.assertEqual(rv.data, FLAC)
        rv.close()
        # Kept, so the next play is a file: seekable, no Deezer involved.
        rv = self.client.get(f"/api/stream/{self.SID}", headers={"Range": "bytes=0-3"})
        self.assertEqual(rv.status_code, 206)
        self.assertEqual(rv.data, b"fLaC")
        rv.close()
        self.assertEqual(self.cdn.hits.count("/route-ok"), 1)
        self.assertEqual(self.queued, [])

    def test_a_stream_refused_on_its_first_bytes_is_this_routes_502(self):
        for name, body, mode in (
            ("route-garbage", encrypt_like_deezer(FLAC, "1"), "length"),
            ("route-expired", b"x", "forbidden"),
        ):
            with self.subTest(name):
                self.queued.clear()
                self._resolve_to(self.cdn.serve(name, body, mode))
                with self.assertLogs("neezer.webui", "WARNING"):
                    rv = self.client.get(f"/api/stream/{self.SID}")
                self.assertEqual(rv.status_code, 502)
                self.assertEqual(rv.get_json(), {"error": "track unavailable"})
                self.assertFalse(os.path.exists(self.track.path))
                # It gets one more go in the background, ahead of bulk work.
                self.assertEqual(self.queued, [(self.SID, self.prefetch_priority)])

    def test_the_response_passes_everything_on_to_the_stream(self):
        from neezer.webui import _then

        def stream(log):
            try:
                yield b"a"
                yield b"b"
            finally:
                log.append("closed")

        started = stream([])
        self.assertEqual(list(_then(next(started), started)), [b"a", b"b"])
        exhausted = stream([])
        list(exhausted)
        self.assertEqual(list(_then(None, exhausted)), [])
        # The client leaves while the block the route read is going out, before
        # the stream is resumed: its cleanup must still run, now.
        log = []
        started = stream(log)
        response = _then(next(started), started)
        next(response)
        response.close()
        self.assertEqual(log, ["closed"])

    def test_a_listener_who_leaves_mid_stream_does_not_queue_a_second_download(self):
        self._resolve_to(self.cdn.serve("route-leave", encrypt_like_deezer(FLAC, self.SID)))
        rv = self.client.get(f"/api/stream/{self.SID}", buffered=False)
        self.assertEqual(rv.status_code, 200)
        # The reader tails the `.part` with read1(), so the FIRST block is however much the
        # download had written when it asked — 2048 bytes on an idle machine, up to 64 KiB
        # under load. What matters is that the block is a PREFIX of the stream (no gap, no
        # garbage), never that it is one particular size.
        first = next(iter(rv.response))
        self.assertTrue(first, "the first read handed nothing on")
        self.assertTrue(
            FLAC.startswith(first),
            f"the first block is not a prefix of the stream ({len(first)} bytes)",
        )
        rv.close()  # the WSGI server, when the client hangs up
        self.assertTrue(
            wait_until(lambda: os.path.exists(self.track.path)),
            "the archive stopped with the listener",
        )
        self.assertTrue(wait_until(lambda: Track[ids.track_uuid(self.SID)].last_modification > 0))
        self.assertEqual(self.queued, [])
        self.assertTrue(
            wait_until(
                lambda: not [
                    f for f in self._files(self.archive_dir) if f.endswith(".part")
                ]
            )
        )

    def test_ready_and_a_seek_land_while_the_listener_stopped_reading(self):
        _need_ffmpeg()
        music = music_flac(self.archive_dir)
        self.cdn.gate.clear()
        self.cdn.rest_sent.clear()
        self._resolve_to(
            self.cdn.serve(
                "route-stall", encrypt_like_deezer(music, self.SID), "gated", head=256 * 1024
            )
        )
        rv = self.client.get(f"/api/stream/{self.SID}", buffered=False)
        self.assertEqual(rv.status_code, 200)
        first = next(iter(rv.response))
        self.assertTrue(first)
        self.cdn.gate.set()
        # From here the listener reads nothing more — a full buffer, a paused tab.
        self.assertTrue(
            wait_until(
                lambda: self.client.get(
                    f"/api/stream/{self.SID}/ready"
                ).get_json()["ready"],
                timeout=25,
            ),
            "ready waited for the listener to read",
        )
        payload = self.client.get(f"/api/stream/{self.SID}/ready").get_json()
        self.assertEqual(payload["ranges"], "bytes")
        self.assertTrue(payload["offset"])
        self.assertEqual(payload["bytes"], os.path.getsize(self.track.path))
        rv.close()
        rv = self.client.get(f"/api/stream/{self.SID}?start=2")
        self.assertEqual(rv.status_code, 200)
        self.assertEqual(rv.headers["X-NS-Start"], "2.000")
        self.assertEqual(rv.headers["Accept-Ranges"], "none")
        self.assertAlmostEqual(decoded_seconds(rv.get_data()), 8.0, delta=0.15)
        self.assertEqual(self.cdn.hits.count("/route-stall"), 1)

    # -- OPUS_320: libopus's ceiling is PER CHANNEL ---------------------------
    # On ffmpeg 9.0.2 `-b:a 320k` is accepted on a stereo source and REFUSED on a
    # mono one ("The bit rate 320000 bps is unsupported. Please choose a value
    # between 500 and 256000.", exit 234). What that cost: an empty response, no
    # cache entry, and on a cold track the live FLAC archive dropped as well.

    def _opus_key(self, bitrate=320):
        from neezer.webui import _transcode_key

        return _transcode_key(Track[ids.track_uuid(self.SID)], bitrate)

    def _archive_flac(self, name, body):
        """One FLAC play: leaves the archive on disk, tagged, as a first play does."""
        self._resolve_to(self.cdn.serve(name, encrypt_like_deezer(body, self.SID), "length"))
        rv = self.client.get(f"/api/stream/{self.SID}")
        self.assertEqual(rv.status_code, 200)
        rv.close()
        # The live producer publishes (tagged, atomically renamed) right after the
        # body ends, so the file's presence is the event, not a sleep.
        self.assertTrue(
            wait_until(lambda: os.path.isfile(self.track.path), timeout=15),
            "the FLAC archive was never published",
        )

    def _transcoded(self, quality="OPUS_320"):
        rv = self.client.get(f"/api/stream/{self.SID}?q={quality}")
        self.assertEqual(rv.status_code, 200)
        body = rv.get_data()
        rv.close()
        return body

    def _opus_info(self, body, name):
        import mutagen.oggopus

        path = Path(self.archive_dir) / name
        path.write_bytes(body)
        return mutagen.oggopus.OggOpus(path).info

    def test_opus_320_of_a_mono_archive_is_clamped_and_cached(self):
        _need_ffmpeg()
        self._archive_flac("route-mono", mono_flac(self.archive_dir))
        body = self._transcoded()
        self.assertTrue(body.startswith(b"OggS"), "not an Ogg Opus stream")
        info = self._opus_info(body, "heard-mono.ogg")
        # One channel in, one channel out: nothing invented a second one.
        self.assertEqual(info.channels, 1)
        self.assertAlmostEqual(info.length, 10.0, delta=0.02)
        # Complete, not truncated — and no ffprobe in the image, so ffmpeg
        # decodes the whole thing (`-i file -f null -`).
        self.assertAlmostEqual(decoded_seconds(body), 10.0, delta=0.05)
        # The rate the file really carries (not the one asked for): measured
        # 241 kbit/s, the 256k libopus accepts on ONE channel. A 320k mono encode
        # would read ~300 and is exactly what the encoder refuses.
        rate = len(body) * 8 / info.length
        self.assertGreater(rate, 200_000)
        self.assertLess(rate, 270_000)
        # ...and it IS the 256k encode: same length, packet for packet, as the
        # same file asked for OPUS_256 (a second cache entry, encoded again).
        self.assertEqual(len(self._transcoded("OPUS_256")), len(body))
        # Cached, so the next play is a file: a byte range, no second encode.
        self.assertTrue(self.app.transcode_cache.has(self._opus_key(320)))
        rv = self.client.get(
            f"/api/stream/{self.SID}?q=OPUS_320", headers={"Range": "bytes=0-3"}
        )
        self.assertEqual(rv.status_code, 206)
        self.assertEqual(rv.data, b"OggS")
        rv.close()
        # A seek into that mono file encodes from the offset: same command, same
        # ceiling, and never cached (a slice is not the track).
        rv = self.client.get(f"/api/stream/{self.SID}?q=OPUS_320&start=2")
        self.assertEqual(rv.status_code, 200)
        self.assertEqual(rv.headers["Accept-Ranges"], "none")
        self.assertAlmostEqual(decoded_seconds(rv.get_data()), 8.0, delta=0.15)
        rv.close()
        # The slice never replaced the cached whole-track transcode.
        self.assertTrue(self.app.transcode_cache.has(self._opus_key(320)))

    def test_opus_320_of_a_stereo_archive_stays_320(self):
        # The clamp must never be a blanket 256: the normal case keeps its rate.
        _need_ffmpeg()
        self._archive_flac("route-stereo", music_flac(self.archive_dir))
        body = self._transcoded()
        info = self._opus_info(body, "heard-stereo.ogg")
        self.assertEqual(info.channels, 2)
        self.assertAlmostEqual(info.length, 10.0, delta=0.02)
        rate = len(body) * 8 / info.length
        # Measured 301 kbit/s for the 320k target on this material (libopus's VBR
        # lands ~94% of it); a 256k clamp would read 240.
        self.assertGreater(rate, 280_000)
        self.assertLess(rate, 360_000)
        self.assertAlmostEqual(decoded_seconds(body), 10.0, delta=0.05)
        self.assertTrue(self.app.transcode_cache.has(self._opus_key(320)))

    def test_opus_320_of_a_cold_mono_track_streams_and_keeps_its_flac(self):
        # The live path has no file to read the layout from: it reads the first
        # block's FLAC STREAMINFO. Getting that wrong is not a degraded stream —
        # ffmpeg never starts, and the archive is dropped with the listener.
        _need_ffmpeg()
        self._resolve_to(
            self.cdn.serve(
                "route-mono-cold", encrypt_like_deezer(mono_flac(self.archive_dir), self.SID), "chunked"
            )
        )
        heard = self._transcoded()
        self.assertTrue(heard.startswith(b"OggS"), "not an Ogg Opus stream")
        info = self._opus_info(heard, "heard-mono-cold.ogg")
        self.assertEqual(info.channels, 1)
        self.assertAlmostEqual(decoded_seconds(heard), 10.0, delta=0.05)
        # The FLAC master still landed: mono, complete, and not re-downloaded.
        audio = mutagen.flac.FLAC(self.track.path)
        self.assertEqual(audio.info.channels, 1)
        self.assertEqual(audio.info.total_samples, 441000)
        self.assertEqual(self.queued, [])
        self.assertEqual(self.cdn.hits.count("/route-mono-cold"), 1)
        # The live transcode is the cache entry, as on the file path.
        self.assertTrue(self.app.transcode_cache.has(self._opus_key(320)))


class SubsonicOpusCommandTestCase(unittest.TestCase):
    """The Subsonic transcode command, run exactly as the image ships it.

    ``docker/default.conf``'s Opus transcoders are command TEMPLATES: one
    ``%outratek`` and no channel count, so they cannot clamp to libopus's 256
    kbit/s per channel the way ``/api/stream`` does. They pin the output to
    stereo instead — measured here, on a mono source, at 320k.
    """

    def _commands(self):
        root = Path(__file__).resolve().parents[1]
        with open(root / "docker" / "default.conf", encoding="utf-8") as fh:
            lines = fh.read().splitlines()
        return [
            line.split("=", 1)[1].strip()
            for line in lines
            if line.startswith(("transcoder_flac_opus", "transcoder_mp3_opus"))
        ]

    @unittest.skipUnless(shutil.which("ffmpeg"), "needs ffmpeg")
    def test_the_shipped_opus_transcoder_takes_320k_on_a_mono_source(self):
        import subprocess

        import mutagen.oggopus

        directory = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, directory, ignore_errors=True)
        source = write_flac(Path(directory) / "mono.flac", 1)
        commands = self._commands()
        self.assertEqual(len(commands), 2, "both Opus transcoders are shipped")
        for i, command in enumerate(commands):
            with self.subTest(command=command):
                # ``%outratek`` is ``%outrate`` plus a literal ``k``: that is how
                # ``media.prepare_transcoding_cmdline`` fills it (it substitutes
                # ``%outrate`` only, so ``%outratek`` becomes "320k").
                argv = [
                    arg.replace("%srcpath", str(source))
                    .replace("%outrate", "320")
                    .replace("%outfmt", "ogg")
                    for arg in shlex.split(command)
                ]
                self.assertTrue("-b:a" in argv and "320k" in argv, argv)
                done = subprocess.run(argv, capture_output=True)
                # Without -ac 2 this is exit 234: "The bit rate 320000 bps is
                # unsupported. Please choose a value between 500 and 256000."
                self.assertEqual(done.returncode, 0, done.stderr.decode()[-300:])
                out = Path(directory) / f"subsonic-{i}.ogg"
                out.write_bytes(done.stdout)
                info = mutagen.oggopus.OggOpus(out).info
                self.assertEqual(info.channels, 2)  # the down-mix that makes it valid
                self.assertAlmostEqual(info.length, 10.0, delta=0.02)
                # Encoded to the 320k budget: measured 263 kbit/s on this
                # (two-identical-channels) material, where 256k reads 240.
                rate = len(done.stdout) * 8 / info.length
                self.assertGreater(rate, 240_000)
                self.assertLess(rate, 400_000)


class EnsureArchivedTestCase(_CdnMixin, TestBase):
    """``ensure_archived``, the path every non-live fetch takes, end to end."""

    def setUp(self):
        super().setUp()
        self.archive_dir = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.archive_dir, ignore_errors=True)
        self.provider = self._provider(self.archive_dir)
        self.provider.fetch_cover = lambda md5, size=1000: None
        patcher = mock.patch("neezer.deezer.analysis.queue_analysis", lambda *a, **k: None)
        patcher.start()
        self.addCleanup(patcher.stop)
        root = library.get_root_folder(self.archive_dir)
        self.track = library.upsert_track(gw_info("1"), root, "FLAC")

    def _resolve_to(self, url):
        self.provider.resolve = lambda sng_id, quality=None: (url, "FLAC", gw_info("1"), "1")

    def test_it_archives_the_audio_deezer_sent(self):
        self._resolve_to(self.cdn.serve("bg-ok", encrypt_like_deezer(FLAC, "1"), "chunked"))
        archive.ensure_archived(self.provider, self.track)
        audio = mutagen.flac.FLAC(self.track.path)
        self.assertEqual(audio.info.total_samples, 214272)
        self.assertEqual(audio["title"], ["Harder (Live)"])
        self.assertGreater(Track[ids.track_uuid("1")].last_modification, 0)

    def test_what_is_not_audio_is_not_archived(self):
        self._resolve_to(self.cdn.serve("bg-garbage", encrypt_like_deezer(FLAC, "916424")))
        with self.assertRaises(DeezerError):
            archive.ensure_archived(self.provider, self.track)
        self.assertEqual(self._files(self.archive_dir), [])
        self.assertEqual(Track[ids.track_uuid("1")].last_modification, 0)


if __name__ == "__main__":
    unittest.main()
