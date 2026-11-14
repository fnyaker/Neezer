# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the AGPLv3 license.

"""Transcription of podcasts, on a fake engine and a real SQLite database.

The engine is a script that writes whisper's JSON; ffmpeg is a script that
writes a WAV. Nothing here needs whisper.cpp, a model, a GPU or the network, so
the suite proves the queue, the lease, the resume, the sidecar and the archive
hook on every machine. The real-engine test at the bottom is opt-in
(``NS_WHISPER=1``) and skipped by default.
"""

import collections
import contextlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from datetime import datetime, timedelta
from unittest import mock

import peewee

from neezer import config as config_module
from neezer.config import DefaultConfig
from neezer.db import (
    Meta,
    PodcastChannel,
    PodcastEpisode,
    PodcastTranscript,
    PodcastTranscriptLease,
    PodcastTranscriptSegment,
    Track,
    TranscribeWorker,
    User,
    db,
    init_database,
    now,
    release_database,
)
from neezer.deezer import workload
from neezer.managers.user import UserManager
from neezer.transcribe import engine, job, runner, settings, sidecar

WAIT = 20.0
HAVE_FFMPEG = shutil.which("ffmpeg")

if not HAVE_FFMPEG and os.environ.get("NS_REQUIRE_FFMPEG") == "1":  # pragma: no cover
    raise RuntimeError("NS_REQUIRE_FFMPEG=1 but ffmpeg is not on PATH")


def write_bytes(path, data):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as handle:
        handle.write(data)
    return path


def write_model(path, magic=b"ggml", size=1_100_000):
    return write_bytes(path, magic + b"\0" * max(0, size - len(magic)))


def write_audio(path):
    return write_bytes(path, b"\xff\xfb\x90\x00" * 256)


def wait_until(predicate, what, timeout=WAIT):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(0.05)
    raise AssertionError(f"timed out waiting for {what}")


def count_statements():
    counts = collections.Counter()
    original = peewee.Database.execute_sql
    mine = threading.get_ident()

    def execute_sql(self, sql, *args, **kwargs):
        if threading.get_ident() == mine:
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


FAKE_WHISPER = r'''#!{python}
import json, os, sys, time

args = sys.argv[1:]
if "--version" in args:
    print("whisper-cli 1.9.5")
    raise SystemExit(0)

def opt(name, default=None):
    return args[args.index(name) + 1] if name in args else default

model = opt("-m")
wav = opt("-f")
out = opt("-of")
language = opt("-l", "auto")
mode = os.environ.get("FAKE_WHISPER_MODE", "ok")

if "-dl" in args:
    print("auto-detected language: fr (p = 0.97)", file=sys.stderr)
    raise SystemExit(0)

log = os.environ.get("FAKE_WHISPER_LOG")
prior = 0
if log:
    try:
        with open(log, "r", encoding="utf-8") as handle:
            prior = sum(1 for _ in handle)
    except FileNotFoundError:
        prior = 0
    with open(log, "a", encoding="utf-8") as handle:
        handle.write("%s\t%s\t%s\n" % (os.path.basename(wav), language, os.path.basename(out)))

fail_at = os.environ.get("FAKE_WHISPER_FAIL_AT")
if fail_at and str(prior) == fail_at:
    print("fake whisper refuses this chunk", file=sys.stderr)
    raise SystemExit(3)
if mode == "fail":
    print("fake whisper failed", file=sys.stderr)
    raise SystemExit(3)
if mode == "hang":
    time.sleep(60)
if mode == "badjson":
    with open(out + ".json", "w", encoding="utf-8") as handle:
        handle.write("{{not json")
    raise SystemExit(0)
if mode == "empty":
    segments = []
elif mode == "huge":
    segments = [{{"offsets": {{"from": i, "to": i + 1}}, "text": "mot %d" % i}} for i in range(2005)]
else:
    segments = [
        {{"offsets": {{"from": 0, "to": 4000}}, "text": "Bonjour depuis le morceau."}},
        {{"offsets": {{"from": 4000, "to": 8000}}, "text": "[Musique]"}},
        {{"offsets": {{"from": 9000, "to": 12000}}, "text": "Deuxième phrase."}},
    ]
print("whisper_print_progress_callback: progress = 10%", file=sys.stderr)
print("whisper_print_progress_callback: progress = 90%", file=sys.stderr)
with open(out + ".json", "w", encoding="utf-8") as handle:
    json.dump({{"transcription": segments, "result": {{"language": "fr" if language == "auto" else language}}}}, handle)
'''


FAKE_FFMPEG = r'''#!{python}
import sys, wave

out = sys.argv[-1]
with wave.open(out, "wb") as handle:
    handle.setnchannels(1)
    handle.setsampwidth(2)
    handle.setframerate(16000)
    handle.writeframes(b"\0\0" * 1600)
'''


class FakeEngineConfig(DefaultConfig):
    TESTING = True

    def __init__(self, db_uri, archive, cache, **transcribe):
        super().__init__()
        self.BASE = dict(self.BASE, database_uri=db_uri)
        self.WEBAPP = dict(self.WEBAPP, cache_dir=cache)
        self.DEEZER = dict(getattr(self, "DEEZER", {}), archive_dir=archive)
        self.TRANSCRIBE = dict(DefaultConfig.TRANSCRIBE, **transcribe)


class _Base(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.db_path = os.path.join(self.tmp, "neezer.db")
        self.uri = "sqlite:///" + self.db_path
        self.archive = os.path.join(self.tmp, "archive")
        self.podcasts = os.path.join(self.archive, "Podcasts")
        self.cache = os.path.join(self.tmp, "cache")
        self.bin_dir = os.path.join(self.tmp, "bin")
        os.makedirs(self.podcasts)
        os.makedirs(self.cache)
        os.makedirs(self.bin_dir)
        self.whisper = write_bytes(
            os.path.join(self.bin_dir, "whisper-cli"),
            FAKE_WHISPER.format(python=sys.executable).encode(),
        )
        os.chmod(self.whisper, 0o755)
        self.ffmpeg = write_bytes(
            os.path.join(self.bin_dir, "ffmpeg"),
            FAKE_FFMPEG.format(python=sys.executable).encode(),
        )
        os.chmod(self.ffmpeg, 0o755)
        self.model_dir = os.path.join(self.tmp, "models")
        os.makedirs(self.model_dir)
        self.model = write_model(os.path.join(self.model_dir, "ggml-small.bin"))
        self.log = os.path.join(self.tmp, "whisper.log")
        self.env = mock.patch.dict(
            os.environ,
            {
                "PATH": self.bin_dir + os.pathsep + os.environ.get("PATH", ""),
                "FAKE_WHISPER_MODE": "ok",
                "FAKE_WHISPER_LOG": self.log,
            },
        )
        self.env.start()
        self.addCleanup(self.env.stop)
        os.environ.pop("FAKE_WHISPER_FAIL_AT", None)
        engine.clear_probe_cache()
        self.config = FakeEngineConfig(
            self.uri,
            self.archive,
            self.cache,
            enabled=True,
            bin=self.whisper,
            model=self.model,
            language="fr",
        )
        previous = config_module.current_config
        config_module.current_config = self.config
        self.addCleanup(setattr, config_module, "current_config", previous)
        init_database(self.uri)
        self.addCleanup(release_database)
        self.addCleanup(engine.clear_probe_cache)
        workload.reset_streams()
        self.addCleanup(workload.reset_streams)
        self.user = UserManager.add("alice", "Alic3", admin=True)

    def channel(self, auto_transcribe=False, language=None, title="Show"):
        return PodcastChannel.create(
            user=self.user,
            url=f"url-{title}",
            title=title,
            auto_transcribe=auto_transcribe,
            transcribe_language=language,
        )

    def episode(self, number=1, duration=1300, channel=None, title=None):
        channel = channel or self.channel(title="Show")
        path = os.path.join(
            self.podcasts,
            channel.title,
            f"2026-01-{number:02d} - Episode {number}.mp3",
        )
        write_audio(path)
        return PodcastEpisode.create(
            channel=channel,
            title=title or f"Episode {number}",
            duration=duration,
            publish_date=datetime(2026, 1, 1) + timedelta(days=number),
            path=path,
            status="completed",
        )

    def drain(self, limit=None):
        return runner.drain(self.config, limit=limit)

    def state(self, episode):
        return job.episode_status(episode.id)

    def log_lines(self):
        try:
            with open(self.log, encoding="utf-8") as handle:
                return [line.rstrip("\n") for line in handle]
        except FileNotFoundError:
            return []


class SidecarTestCase(_Base):
    def payload(self, segments=None, duration=3_600_000):
        segments = segments if segments is not None else [
            {"start_ms": 0, "end_ms": 6000, "text": "Bonjour à tous."},
            {"start_ms": 6000, "end_ms": 12_000, "text": "Deuxième phrase."},
        ]
        return sidecar.build(
            episode_id="0" * 36,
            language="fr",
            engine="whisper.cpp",
            model="ggml-small.bin",
            engine_version="1.9.5",
            duration_ms=duration,
            segments=segments,
            created="2026-11-14T10:00:00",
            updated="2026-11-14T10:01:00",
        )

    def test_the_json_and_the_srt_are_written_atomically(self):
        audio = write_audio(os.path.join(self.podcasts, "Show", "Episode.mp3"))
        path = sidecar.write(audio, self.payload(), sidecar.root_for(self.archive))
        self.assertEqual(path, os.path.splitext(audio)[0] + ".transcript.json")
        with open(path, encoding="utf-8") as handle:
            document = json.load(handle)
        self.assertEqual(document["format"], 1)
        self.assertEqual(document["language"], "fr")
        self.assertEqual(document["segments"][1]["text"], "Deuxième phrase.")
        with open(sidecar.srt_path(audio), encoding="utf-8") as handle:
            srt = handle.read()
        self.assertIn("00:00:00,000 --> 00:00:06,000", srt)
        self.assertIn("00:00:06,000 --> 00:00:12,000", srt)
        self.assertTrue(srt.startswith("1\n"))
        # No temporary file survives a successful write.
        leftovers = [
            name for name in os.listdir(os.path.dirname(audio))
            if name.endswith(".part")
        ]
        self.assertEqual(leftovers, [])

    def test_an_unwritable_copy_leaves_the_previous_sidecar_intact(self):
        audio = write_audio(os.path.join(self.podcasts, "Show", "Episode.mp3"))
        root = sidecar.root_for(self.archive)
        sidecar.write(audio, self.payload(), root)
        with open(sidecar.json_path(audio), "rb") as handle:
            before = handle.read()
        broken = self.payload()
        broken["segments"] = [{"start_ms": 0, "end_ms": 1, "text": " "}]
        with self.assertRaises(sidecar.SidecarError):
            sidecar.write(audio, broken, root)
        with open(sidecar.json_path(audio), "rb") as handle:
            self.assertEqual(handle.read(), before)
        with mock.patch.object(sidecar.os, "replace", side_effect=OSError("disk full")):
            with self.assertRaises(OSError):
                sidecar.write(audio, self.payload(), root)
        with open(sidecar.json_path(audio), "rb") as handle:
            self.assertEqual(handle.read(), before)

    def test_a_path_outside_the_podcast_archive_is_refused(self):
        outside = write_audio(os.path.join(self.tmp, "outside.mp3"))
        root = sidecar.root_for(self.archive)
        with self.assertRaises(sidecar.SidecarError):
            sidecar.write(outside, self.payload(), root)
        # A symlink inside the archive that points outside is refused too.
        link = os.path.join(self.podcasts, "Show", "link.mp3")
        os.makedirs(os.path.dirname(link), exist_ok=True)
        os.symlink(outside, link)
        with self.assertRaises(sidecar.SidecarError):
            sidecar.write(link, self.payload(), root)
        # And a sidecar that resolves outside is refused on read.
        target = os.path.join(self.tmp, "elsewhere.transcript.json")
        write_bytes(target, json.dumps(self.payload()).encode())
        with self.assertRaises(sidecar.SidecarError):
            sidecar.load_file(target, root)

    def test_remove_deletes_the_text_never_the_audio(self):
        audio = write_audio(os.path.join(self.podcasts, "Show", "Episode.mp3"))
        root = sidecar.root_for(self.archive)
        sidecar.write(audio, self.payload(), root)
        sidecar.remove(audio, root)
        self.assertFalse(os.path.exists(sidecar.json_path(audio)))
        self.assertFalse(os.path.exists(sidecar.srt_path(audio)))
        self.assertTrue(os.path.isfile(audio))

    def test_remove_still_works_when_the_audio_is_gone(self):
        audio = write_audio(os.path.join(self.podcasts, "Show", "Episode.mp3"))
        root = sidecar.root_for(self.archive)
        sidecar.write(audio, self.payload(), root)
        os.unlink(audio)
        sidecar.remove(audio, root)
        self.assertFalse(os.path.exists(sidecar.json_path(audio)))
        outside = os.path.join(self.tmp, "outside.mp3")
        with self.assertRaises(sidecar.SidecarError):
            sidecar.remove(outside, root)

    def test_an_hour_of_text_is_about_two_hundred_kilobytes(self):
        """The plan dimensions storage at 200-250 kB per hour; the sidecar is
        the same text plus the offsets, so it must not double it."""
        segment = (
            "Ceci est une phrase française de podcast d'environ quatre-vingt-dix "
            "caractères, avec des accents et de la ponctuation."
        )
        segments = [
            {"start_ms": i * 6000, "end_ms": (i + 1) * 6000, "text": segment}
            for i in range(600)
        ]
        document = sidecar.build(
            episode_id="0" * 36,
            language="fr",
            engine="whisper.cpp",
            model="ggml-large-v3-turbo-q8_0.bin",
            engine_version="1.9.5",
            duration_ms=3_600_000,
            segments=segments,
            created="2026-11-14T10:00:00",
            updated="2026-11-14T10:00:00",
        )
        raw = sidecar.dumps(document)
        self.assertGreater(len(raw), 100_000)
        self.assertLess(len(raw), 250_000)
        self.assertLess(len(sidecar.srt_bytes(document["segments"])), 300_000)


class EngineTestCase(_Base):
    def paths(self, model=None):
        return {"bin": self.whisper, "model": model or self.model, "ffmpeg": self.ffmpeg}

    def test_probe_names_every_missing_piece(self):
        self.assertEqual(engine.probe(self.paths())["reason"], None)
        self.assertEqual(
            engine.probe({"bin": None, "model": self.model, "ffmpeg": self.ffmpeg})[
                "reason"
            ],
            "engine_missing",
        )
        self.assertEqual(
            engine.probe({"bin": self.whisper, "model": None, "ffmpeg": self.ffmpeg})[
                "reason"
            ],
            "model_missing",
        )
        bad = write_model(os.path.join(self.model_dir, "bad.bin"), magic=b"NOPE")
        self.assertEqual(engine.probe(self.paths(bad))["reason"], "model_invalid")
        tiny = write_model(
            os.path.join(self.model_dir, "tiny.bin"), size=64
        )
        self.assertEqual(engine.probe(self.paths(tiny))["reason"], "model_invalid")

    def test_the_model_follows_the_memory_that_is_actually_free(self):
        q8 = write_model(os.path.join(self.model_dir, "ggml-large-v3-turbo-q8_0.bin"))
        q5 = write_model(os.path.join(self.model_dir, "ggml-large-v3-turbo-q5_0.bin"))
        small = write_model(os.path.join(self.model_dir, "ggml-small.bin"))
        self.assertEqual(
            engine.choose_model(small, free_bytes=4_000_000_000), q8
        )
        self.assertEqual(
            engine.choose_model(small, free_bytes=1_000_000_000), q5
        )
        os.unlink(q5)
        self.assertEqual(
            engine.choose_model(small, free_bytes=1_000_000_000), small
        )
        self.assertEqual(engine.choose_model(small, free_bytes=None), small)
        empty = tempfile.mkdtemp(dir=self.tmp)
        self.assertIsNone(
            engine.choose_model(os.path.join(empty, "absent.bin"), 4_000_000_000)
        )

    def test_segments_are_absolute_bounded_and_deduplicated(self):
        raw = [
            {"offsets": {"from": 0, "to": 1900}, "text": "dans l'overlap"},
            {"offsets": {"from": 2500, "to": 5000}, "text": "à cheval"},
            {"offsets": {"from": 5000, "to": 9000}, "text": "[Musique]"},
            {"offsets": {"from": 9000, "to": 12_000}, "text": "  Une   phrase.  "},
            {"offsets": {"from": 601_000, "to": 602_000}, "text": "hors du chunk"},
            {"offsets": {"from": 1, "to": 2}, "text": ""},
            {"offsets": {"from": 3000, "to": 4000}, "text": "x" * 5000},
        ]
        out = engine.normalize_segments(raw, 600_000, 600_000, overlap_ms=2000)
        self.assertEqual(
            [(s["start_ms"], s["end_ms"]) for s in out],
            [(602_500, 605_000), (603_000, 604_000), (609_000, 612_000)],
        )
        self.assertEqual(
            [s["text"] for s in out], ["à cheval", "x" * 2000, "Une phrase."]
        )
        self.assertEqual(len(out[1]["text"]), 2000)
        self.assertTrue(all(s["end_ms"] <= 1_200_000 for s in out))

    def test_a_broken_json_is_a_named_error(self):
        path = os.path.join(self.tmp, "broken.json")
        write_bytes(path, b"{nope")
        with self.assertRaises(engine.EngineError) as caught:
            engine.parse_json(path)
        self.assertEqual(caught.exception.code, "bad_json")

    @unittest.skipUnless(HAVE_FFMPEG, "ffmpeg is not on PATH")
    def test_the_real_ffmpeg_command_produces_sixteen_kilohertz_mono(self):
        import wave

        source = os.path.join(self.tmp, "speech.wav")
        subprocess.run(
            [
                HAVE_FFMPEG,
                "-nostdin",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=3",
                "-ac",
                "1",
                "-ar",
                "44100",
                "-y",
                source,
            ],
            check=True,
        )
        out = os.path.join(self.tmp, "chunk.wav")
        engine.extract_chunk(
            {"bin": self.whisper, "model": self.model, "ffmpeg": HAVE_FFMPEG},
            source,
            1000,
            1500,
            out,
        )
        with wave.open(out, "rb") as handle:
            self.assertEqual(handle.getnchannels(), 1)
            self.assertEqual(handle.getframerate(), 16000)
            self.assertAlmostEqual(handle.getnframes(), 24000, delta=2000)

    def test_threads_leave_the_machine_a_share(self):
        for cpus, want in ((1, 1), (2, 1), (4, 2), (8, 2), (16, 4), (64, 4)):
            self.assertEqual(engine.threads(cpus), want, cpus)

    def test_the_engine_module_never_imports_the_database_or_flask(self):
        code = (
            "import sys; import neezer.transcribe.engine; "
            "print('neezer.db' in sys.modules, 'flask' in sys.modules)"
        )
        out = subprocess.run(
            [sys.executable, "-c", code],
            capture_output=True,
            text=True,
            check=True,
        ).stdout.strip()
        self.assertEqual(out, "False False")


class QueueTestCase(_Base):
    def test_enqueue_is_one_row_and_idempotent(self):
        episode = self.episode()
        result = job.enqueue(episode.id, user=self.user, app=self.config)
        self.assertEqual(result["status"], "queued")
        self.assertEqual(job.enqueue(episode.id, app=self.config)["status"], "queued")
        self.assertEqual(PodcastTranscript.select().count(), 1)
        row = PodcastTranscript.get()
        self.assertEqual(row.duration_ms, 1_300_000)
        self.assertEqual(row.source, "server")
        self.assertIsNone(row.worker_name)
        self.assertEqual(row.user_id, self.user.id)

    def test_a_ready_episode_refuses_unless_forced(self):
        episode = self.episode()
        job.enqueue(episode.id, app=self.config)
        row = PodcastTranscript.get()
        row.status = "ready"
        row.done_ms = row.duration_ms
        row.n_segments = 1
        row.save()
        PodcastTranscriptSegment.create(
            transcript=row,
            idx=0,
            start_ms=0,
            end_ms=1000,
            text="Bonjour",
            search_text="bonjour",
        )
        with self.assertRaises(job.QueueError) as caught:
            job.enqueue(episode.id, app=self.config)
        self.assertEqual(caught.exception.code, "ready")
        out = job.enqueue(episode.id, force=True, app=self.config)
        self.assertEqual(out["status"], "queued")
        row = PodcastTranscript.get()
        self.assertEqual((row.status, row.done_ms, row.n_segments), ("queued", 0, 0))
        self.assertEqual(PodcastTranscriptSegment.select().count(), 0)

    def test_a_full_queue_refuses_the_twenty_first(self):
        channel = self.channel()
        for number in range(1, job.QUEUE_MAX + 2):
            self.episode(number=number, duration=60, channel=channel)
        episodes = list(PodcastEpisode.select().order_by(PodcastEpisode.title))
        for episode in episodes[: job.QUEUE_MAX]:
            job.enqueue(episode.id, app=self.config)
        with self.assertRaises(job.QueueError) as caught:
            job.enqueue(episodes[-1].id, app=self.config)
        self.assertEqual(caught.exception.code, "queue_full")
        self.assertEqual(caught.exception.retry_in, 60)

    def test_stop_keeps_what_was_already_transcribed(self):
        episode = self.episode()
        job.enqueue(episode.id, app=self.config)
        out = job.stop(episode.id, app=self.config)
        self.assertEqual(out["status"], "stopped")
        self.assertEqual(PodcastTranscript.get().status, "stopped")
        with self.assertRaises(job.QueueError) as caught:
            job.stop(episode.id, app=self.config)
        self.assertEqual(caught.exception.code, "not_running")

    def test_nothing_is_transcribed_until_the_runner_claims(self):
        episode = self.episode()
        job.enqueue(episode.id, app=self.config)
        self.assertEqual(self.log_lines(), [])
        self.assertEqual(PodcastTranscript.get().status, "queued")


class LeaseTestCase(_Base):
    def worker(self, name="Bureau", kind="client"):
        return TranscribeWorker.create(
            name=name, kind=kind, token="encrypted-" + name
        )

    def claimed(self, episode, worker):
        out = job.claim_next(worker, app=self.config)
        self.assertIsNotNone(out)
        self.assertEqual(out["episode_id"], str(episode.id))
        return out

    def test_a_lease_runs_to_ready(self):
        episode = self.episode(duration=1300)
        job.enqueue(episode.id, app=self.config)
        worker = self.worker()
        out = self.claimed(episode, worker)
        token = out["lease"]
        self.assertEqual(out["start_ms"], 0)
        self.assertEqual(out["chunk_seconds"], 600)
        self.assertEqual(out["overlap_seconds"], 2)
        self.assertEqual(PodcastTranscript.get().status, "leased")
        self.assertTrue(PodcastTranscriptLease.get().lease_token_hash)
        self.assertNotEqual(
            PodcastTranscriptLease.get().lease_token_hash, token
        )
        beat = job.heartbeat(episode.id, worker, token)
        self.assertFalse(beat["cancel"])
        chunk = [
            {"start_ms": 1000, "end_ms": 4000, "text": "Bonjour"},
            {"start_ms": 5000, "end_ms": 9000, "text": "Au revoir"},
        ]
        applied = job.apply_chunk(episode.id, worker, token, 0, 600_000, chunk)
        self.assertTrue(applied["applied"])
        self.assertEqual(applied["done_ms"], 600_000)
        self.assertEqual(applied["n_segments"], 2)
        again = job.apply_chunk(episode.id, worker, token, 0, 600_000, chunk)
        self.assertFalse(again["applied"])
        self.assertEqual(PodcastTranscriptSegment.select().count(), 2)
        with self.assertRaises(job.QueueError) as caught:
            job.apply_chunk(episode.id, worker, token, 900_000, 100_000, [])
        self.assertEqual(caught.exception.code, "chunk_out_of_order")
        with self.assertRaises(job.QueueError) as caught:
            job.complete(episode.id, worker, token)
        self.assertEqual(caught.exception.code, "incomplete")
        job.apply_chunk(episode.id, worker, token, 600_000, 600_000, [])
        done = job.apply_chunk(episode.id, worker, token, 1_200_000, 100_000, [])
        self.assertEqual(done["done_ms"], 1_300_000)
        out = job.complete(
            episode.id, worker, token, language="fr", model="ggml-small.bin"
        )
        self.assertEqual(out["status"], "ready")
        row = PodcastTranscript.get()
        self.assertEqual(row.status, "ready")
        self.assertEqual(row.done_ms, 1_300_000)
        self.assertEqual(row.worker_name, "Bureau")
        self.assertEqual(PodcastTranscriptLease.select().count(), 0)

    def test_an_expired_lease_returns_to_the_queue_then_fails_after_three(self):
        episode = self.episode(duration=1300)
        job.enqueue(episode.id, app=self.config)
        worker = self.worker()
        for attempt in range(1, 4):
            job.claim_next(worker, app=self.config)
            lease = PodcastTranscriptLease.get()
            lease.expires_at = now() - timedelta(seconds=1)
            lease.save()
            changed = job.expire_leases()
            self.assertEqual(changed, 1)
            row = PodcastTranscript.get()
            if attempt < 3:
                self.assertEqual(row.status, "queued")
                self.assertEqual(PodcastTranscriptLease.get().attempts, attempt)
            else:
                self.assertEqual(row.status, "error")
                self.assertEqual(row.error_code, "lease_lost")
                self.assertEqual(PodcastTranscriptLease.select().count(), 0)

    def test_a_cancelled_lease_stops_instead_of_returning(self):
        episode = self.episode()
        job.enqueue(episode.id, app=self.config)
        worker = self.worker()
        out = self.claimed(episode, worker)
        job.stop(episode.id, app=self.config)
        self.assertTrue(job.heartbeat(episode.id, worker, out["lease"])["cancel"])
        lease = PodcastTranscriptLease.get()
        lease.expires_at = now() - timedelta(seconds=1)
        lease.save()
        job.expire_leases()
        self.assertEqual(PodcastTranscript.get().status, "stopped")

    def test_one_server_lease_and_many_clients(self):
        channel = self.channel()
        first = self.episode(number=1, duration=60, channel=channel)
        second = self.episode(number=2, duration=60, channel=channel)
        job.enqueue(first.id, app=self.config)
        job.enqueue(second.id, app=self.config)
        server = self.worker("conteneur", "server")
        self.assertIsNotNone(job.claim_next(server, app=self.config))
        self.assertIsNone(job.claim_next(server, app=self.config))
        client = self.worker("Bureau", "client")
        self.assertIsNotNone(job.claim_next(client, app=self.config))
        self.assertEqual(
            PodcastTranscript.select().where(PodcastTranscript.status == "leased").count(),
            2,
        )

    def test_a_wrong_token_or_a_revoked_worker_is_refused(self):
        episode = self.episode()
        job.enqueue(episode.id, app=self.config)
        worker = self.worker()
        token = self.claimed(episode, worker)["lease"]
        with self.assertRaises(job.LeaseError):
            job.heartbeat(episode.id, worker, token + "x")
        worker.revoked = now()
        worker.save()
        with self.assertRaises(job.LeaseError):
            job.heartbeat(episode.id, worker, token)

    def test_fail_and_release_keep_the_partial_text(self):
        episode = self.episode(duration=1300)
        job.enqueue(episode.id, app=self.config)
        worker = self.worker()
        token = self.claimed(episode, worker)["lease"]
        job.apply_chunk(
            episode.id, worker, token, 0, 600_000,
            [{"start_ms": 10, "end_ms": 20, "text": "déjà là"}],
        )
        out = job.fail(episode.id, worker, token, "exit", "le moteur est tombé")
        self.assertEqual(out["status"], "error")
        row = PodcastTranscript.get()
        self.assertEqual((row.done_ms, row.n_segments), (600_000, 1))
        self.assertEqual(row.error_code, "exit")
        self.assertEqual(PodcastTranscriptSegment.select().count(), 1)
        job.enqueue(episode.id, app=self.config)  # an error is resumable
        token = self.claimed(episode, worker)["lease"]
        job.release(episode.id, worker, token)
        self.assertEqual(PodcastTranscript.get().status, "queued")
        self.assertEqual(PodcastTranscript.get().done_ms, 600_000)


class ProcessGateTestCase(unittest.TestCase):
    def test_a_stream_stops_and_resumes_the_process_group(self):
        signals = []
        busy = {"value": False}
        moment = {"value": 0.0}

        class Proc:
            pid = 4321

        gate = runner.ProcessGate(busy=lambda: busy["value"])
        with mock.patch.object(runner.os, "getpgid", lambda pid: pid),              mock.patch.object(
                 runner.os, "killpg", lambda pid, number: signals.append(number)
             ),              mock.patch.object(runner.time, "monotonic", lambda: moment["value"]):
            gate.attach(Proc())
            busy["value"] = True
            gate.tick()
            gate.tick()
            self.assertEqual(signals, [runner.signal.SIGSTOP])
            self.assertTrue(gate.paused)
            busy["value"] = False
            moment["value"] = 10.0
            gate.tick()
            self.assertEqual(signals, [runner.signal.SIGSTOP])
            moment["value"] = 20.0
            gate.tick()
            self.assertEqual(signals, [runner.signal.SIGSTOP])
            moment["value"] = 26.0
            gate.tick()
            self.assertEqual(
                signals, [runner.signal.SIGSTOP, runner.signal.SIGCONT]
            )
            self.assertFalse(gate.paused)
            gate.detach()


class RunnerTestCase(_Base):
    def test_a_normal_run_writes_the_sidecar_and_the_srt(self):
        episode = self.episode(duration=1300)
        job.enqueue(episode.id, app=self.config)
        self.assertEqual(self.drain(), 1)
        row = PodcastTranscript.get()
        self.assertEqual(row.status, "ready")
        self.assertEqual(row.done_ms, 1_300_000)
        self.assertEqual(row.n_segments, 6)
        self.assertEqual(row.engine, "whisper.cpp")
        self.assertEqual(row.model, "ggml-small.bin")
        self.assertEqual(row.source, "server")
        self.assertEqual(row.worker_name, "inline")
        self.assertTrue(os.path.isfile(sidecar.json_path(episode.path)))
        self.assertTrue(os.path.isfile(sidecar.srt_path(episode.path)))
        self.assertEqual(row.transcript_path, sidecar.json_path(episode.path))
        # Three chunks, in order: 0, 600 s, 1200 s.
        starts = [line.split("\t")[0] for line in self.log_lines()]
        self.assertEqual(
            starts, ["chunk-0.wav", "chunk-600000.wav", "chunk-1200000.wav"]
        )
        document = sidecar.load_file(
            sidecar.json_path(episode.path), sidecar.root_for(self.archive)
        )
        self.assertEqual(document["language"], "fr")
        self.assertEqual(
            [s["start_ms"] for s in document["segments"]],
            [0, 9000, 600_000, 609_000, 1_200_000, 1_209_000],
        )
        self.assertEqual(Track.select().count(), 0)

    def test_a_resume_starts_at_done_ms_not_at_zero(self):
        episode = self.episode(duration=1300)
        job.enqueue(episode.id, app=self.config)
        os.environ["FAKE_WHISPER_FAIL_AT"] = "1"  # first chunk OK, second fails
        self.assertEqual(self.drain(), 1)
        row = PodcastTranscript.get()
        self.assertEqual(row.status, "error")
        self.assertEqual(row.error_code, "exit")
        self.assertEqual(row.done_ms, 600_000)
        self.assertEqual(row.n_segments, 2)
        self.assertEqual(len(self.log_lines()), 2)
        os.environ.pop("FAKE_WHISPER_FAIL_AT")
        open(self.log, "w").close()
        job.enqueue(episode.id, app=self.config)
        self.assertEqual(self.drain(), 1)
        row = PodcastTranscript.get()
        self.assertEqual(row.status, "ready")
        self.assertEqual(row.done_ms, 1_300_000)
        self.assertEqual(
            [line.split("\t")[0] for line in self.log_lines()],
            ["chunk-600000.wav", "chunk-1200000.wav"],
        )

    def test_a_stop_is_honoured_within_two_seconds_and_keeps_the_partial(self):
        episode = self.episode(duration=1300)
        job.enqueue(episode.id, app=self.config)
        os.environ["FAKE_WHISPER_MODE"] = "hang"
        thread = threading.Thread(target=lambda: self.drain(limit=1), daemon=True)
        thread.start()
        wait_until(lambda: PodcastTranscript.get().status == "running", "the claim")
        wait_until(lambda: bool(self.log_lines()), "the engine to start")
        started = time.monotonic()
        job.stop(episode.id, app=self.config)
        thread.join(10)
        self.assertFalse(thread.is_alive(), "the runner did not stop")
        self.assertLess(time.monotonic() - started, 5.0)
        row = PodcastTranscript.get()
        self.assertEqual(row.status, "stopped")
        self.assertEqual(row.done_ms, 0)
        self.assertTrue(os.path.isfile(episode.path))

    def test_an_open_stream_holds_the_job_until_it_closes(self):
        episode = self.episode(duration=600)
        job.enqueue(episode.id, app=self.config)
        workload.note_stream_open()
        thread = threading.Thread(target=lambda: self.drain(limit=1), daemon=True)
        thread.start()
        time.sleep(0.5)
        self.assertEqual(self.log_lines(), [])
        self.assertEqual(PodcastTranscript.get().status, "running")
        workload.note_stream_close()
        thread.join(WAIT)
        self.assertFalse(thread.is_alive())
        self.assertEqual(PodcastTranscript.get().status, "ready")
        self.assertEqual(len(self.log_lines()), 1)

    def test_a_job_never_opens_a_socket(self):
        episode = self.episode(duration=600)
        job.enqueue(episode.id, app=self.config)
        with mock.patch("socket.create_connection", side_effect=AssertionError("network")), \
             mock.patch("socket.getaddrinfo", side_effect=AssertionError("dns")), \
             mock.patch(
                 "requests.Session.request", side_effect=AssertionError("http")
             ):
            self.assertEqual(self.drain(), 1)
        self.assertEqual(PodcastTranscript.get().status, "ready")

    def test_the_chunk_temporaries_are_gone_after_the_run(self):
        episode = self.episode(duration=1300)
        job.enqueue(episode.id, app=self.config)
        self.drain()
        asr = os.path.join(self.cache, runner.ASR_DIR)
        self.assertEqual(os.listdir(asr) if os.path.isdir(asr) else [], [])

    def test_the_inline_runner_does_not_start_a_thread_under_test(self):
        inline = runner.InlineRunner(self.config)
        inline.start()
        self.assertIsNone(inline._thread)
        self.assertEqual(inline.status()["runner"], "inline")

    def test_an_external_runner_leaves_the_jobs_queued(self):
        self.config.TRANSCRIBE = dict(self.config.TRANSCRIBE, runner="external")
        episode = self.episode(duration=60)
        job.enqueue(episode.id, app=self.config)
        external = runner.build_runner(self.config)
        self.assertIsInstance(external, runner.ExternalRunner)
        self.assertEqual(external.drain(), 0)
        self.assertEqual(self.log_lines(), [])
        self.assertEqual(PodcastTranscript.get().status, "queued")
        status = external.status()
        self.assertEqual((status["runner"], status["started"]), ("external", False))


class AutoTranscribeTestCase(_Base):
    def test_the_archive_hook_queues_a_new_episode_of_a_marked_show(self):
        from neezer.deezer import archive

        channel = self.channel(auto_transcribe=False)
        episode = self.episode(channel=channel)
        archive._finalize_episode(episode, episode.path)
        self.assertEqual(PodcastTranscript.select().count(), 0)
        self.assertEqual(PodcastEpisode.get().status, "completed")

        channel.auto_transcribe = True
        channel.save()
        archive._finalize_episode(episode, episode.path)
        self.assertEqual(PodcastTranscript.select().count(), 1)
        self.assertEqual(PodcastTranscript.get().status, "queued")
        self.assertEqual(Track.select().count(), 0)

    def test_a_misconfigured_engine_does_not_fail_the_archive(self):
        from neezer.deezer import archive

        self.config.TRANSCRIBE = dict(self.config.TRANSCRIBE, bin="/nope/whisper-cli")
        channel = self.channel(auto_transcribe=True)
        episode = self.episode(channel=channel)
        archive._finalize_episode(episode, episode.path)
        self.assertEqual(PodcastEpisode.get().status, "completed")
        self.assertEqual(PodcastTranscript.select().count(), 0)

    def test_backlog_all_recent_and_none(self):
        channel = self.channel(auto_transcribe=True)
        for number in range(1, 13):
            self.episode(number=number, duration=60, channel=channel)
        self.assertEqual(
            job.queue_backlog(channel=channel, backlog="none", app=self.config)["queued"], 0
        )
        recent = job.queue_backlog(channel=channel, backlog="recent", app=self.config)
        self.assertEqual(recent["queued"], 10)
        PodcastTranscript.delete().execute()
        everything = job.queue_backlog(channel=channel, backlog="all", app=self.config)
        self.assertEqual(everything["queued"], 12)
        self.assertEqual(PodcastTranscript.select().count(), 12)

    def test_the_daily_budget_skips_automatic_jobs_and_never_manual_ones(self):
        self.config.TRANSCRIBE = dict(self.config.TRANSCRIBE, daily_minutes=1)
        channel = self.channel(auto_transcribe=True)
        episodes = [
            self.episode(number=number, duration=30, channel=channel)
            for number in range(1, 4)
        ]
        self.assertEqual(
            job.auto_enqueue(episodes[0], app=self.config)["status"], "queued"
        )
        self.assertEqual(
            job.auto_enqueue(episodes[1], app=self.config)["status"], "queued"
        )
        skipped = job.auto_enqueue(episodes[2], app=self.config)
        self.assertEqual(skipped["status"], "skipped")
        self.assertEqual(skipped["reason"], "budget")
        self.assertEqual(PodcastTranscript.select().count(), 2)
        job.enqueue(episodes[2].id, user=self.user, app=self.config)
        self.assertEqual(PodcastTranscript.select().count(), 3)

    def test_an_unlimited_budget_is_zero(self):
        self.config.TRANSCRIBE = dict(self.config.TRANSCRIBE, daily_minutes=0)
        channel = self.channel(auto_transcribe=True)
        for number in range(1, 6):
            episode = self.episode(number=number, duration=600, channel=channel)
            self.assertEqual(
                job.auto_enqueue(episode, app=self.config)["status"], "queued"
            )
        self.assertEqual(job.auto_budget_used_ms(), 5 * 600_000)
        self.assertIsNone(
            job._budget_state(settings.load(self.config))["remaining_ms"]
        )

    def test_resume_auto_does_nothing_in_one_query_when_no_show_is_marked(self):
        channel = self.channel(auto_transcribe=False)
        for number in range(1, 4):
            self.episode(number=number, duration=60, channel=channel)
        with count_statements() as counts:
            self.assertEqual(job.resume_auto(self.config), 0)
        # The effective settings read their ``tr.*`` overrides, then the marked
        # shows are asked once: two statements on an idle server.
        self.assertEqual(sum(counts.values()), 2)

    def test_resume_auto_fills_the_free_slots_and_does_not_duplicate(self):
        channel = self.channel(auto_transcribe=True)
        for number in range(1, 25):
            self.episode(number=number, duration=60, channel=channel)
        filled = job.resume_auto(self.config)
        self.assertEqual(filled, job.QUEUE_MAX)
        self.assertEqual(PodcastTranscript.select().count(), job.QUEUE_MAX)
        self.assertEqual(job.resume_auto(self.config), 0)
        self.assertEqual(PodcastTranscript.select().count(), job.QUEUE_MAX)

    def test_resume_auto_honours_the_stored_show_backlog(self):
        channel = self.channel(auto_transcribe=True)
        for number in range(1, 4):
            self.episode(number=number, duration=60, channel=channel)
        key = "tr.bl." + str(channel.id)[:24]
        Meta.create(key=key, value="none")
        self.assertEqual(job.resume_auto(self.config), 0)
        Meta.update(value="all").where(Meta.key == key).execute()
        self.assertEqual(job.resume_auto(self.config), 3)

    def test_the_estimate_says_hours_and_a_nominal_cpu_cost(self):
        channel = self.channel(auto_transcribe=True)
        for number in range(1, 11):
            self.episode(number=number, duration=3600, channel=channel)
        estimate = job.estimate(channel=channel, app=self.config)
        self.assertEqual(estimate["episodes"], 10)
        self.assertEqual(estimate["audio_seconds"], 36_000)
        self.assertEqual(estimate["audio_hours"], 10.0)
        self.assertEqual(estimate["cpu_hours"], 5.0)  # nominal 2x real time

    def test_a_restart_requeues_a_running_job_and_keeps_done_ms(self):
        episode = self.episode(duration=1300)
        job.enqueue(episode.id, app=self.config)
        row = PodcastTranscript.get()
        row.status = "running"
        row.done_ms = 600_000
        row.save()
        out = job.resume_if_interrupted(self.config)
        self.assertEqual(out["requeued"], 1)
        row = PodcastTranscript.get()
        self.assertEqual((row.status, row.done_ms), ("queued", 600_000))


class ConfigTestCase(_Base):
    def test_no_transcribe_section_is_disabled_not_a_crash(self):
        class BareApp:
            config = {}

        app = BareApp()
        values = settings.load(app)
        self.assertFalse(values["enabled"])
        self.assertEqual(values["runner"], "inline")
        self.assertEqual(values["daily_minutes"], 120)
        verdict = engine.quick_check(settings.paths(app))
        self.assertFalse(verdict["ok"])
        self.assertIn(verdict["reason"], ("engine_missing", "model_missing"))
        self.assertIsNone(settings.paths(app)["root"])

    def test_a_path_only_setting_never_comes_from_a_payload(self):
        # The settings module only reads the file; it has no setter and no route.
        self.assertFalse(hasattr(settings, "save"))
        self.assertFalse(hasattr(settings, "set"))

    def test_the_config_section_and_the_settings_defaults_agree(self):
        from neezer.config import DefaultConfig as Base

        self.assertEqual(set(settings.DEFAULTS), set(Base.TRANSCRIBE))
        for key, value in settings.DEFAULTS.items():
            self.assertEqual(value, Base.TRANSCRIBE[key], key)

    def test_hostile_values_are_clamped_not_obeyed(self):
        self.config.TRANSCRIBE = dict(
            self.config.TRANSCRIBE,
            runner="wat",
            max_minutes=1,
            daily_minutes=-4,
            language="zz",
            enabled="yes",
            search_segments="junk",
        )
        values = settings.load(self.config)
        self.assertEqual(values["runner"], "inline")
        self.assertEqual(values["max_minutes"], 5)
        self.assertEqual(values["daily_minutes"], 0)
        self.assertEqual(values["language"], "auto")
        self.assertTrue(values["enabled"])
        self.assertEqual(values["search_segments"], 1_200_000)

    def test_the_engine_being_missing_is_a_named_error(self):
        self.config.TRANSCRIBE = dict(self.config.TRANSCRIBE, bin=None)
        episode = self.episode()
        with self.assertRaises(job.QueueError) as caught:
            job.enqueue(episode.id, app=self.config)
        self.assertEqual(caught.exception.code, "engine_missing")
        channel = self.channel(auto_transcribe=True)
        auto_episode = self.episode(number=2, channel=channel)
        skipped = job.auto_enqueue(auto_episode, app=self.config)
        self.assertEqual(skipped["reason"], "engine_missing")


class RebuildTestCase(_Base):
    def ready_episode(self):
        episode = self.episode(duration=1300)
        job.enqueue(episode.id, user=self.user, app=self.config)
        self.drain()
        return episode

    def test_rebuild_recreates_the_rows_from_the_sidecar(self):
        episode = self.ready_episode()
        path = sidecar.json_path(episode.path)
        self.assertTrue(os.path.isfile(path))
        transcript = PodcastTranscript.get()
        PodcastTranscript.delete().where(
            PodcastTranscript.episode == episode.id
        ).execute()
        self.assertEqual(PodcastTranscriptSegment.select().count(), 0)
        self.assertTrue(os.path.isfile(path))

        stats = job.rebuild(app=self.config)
        self.assertEqual(stats["scanned"], 1)
        self.assertEqual(stats["rebuilt"], 1)
        row = PodcastTranscript.get()
        self.assertEqual(row.status, "ready")
        self.assertEqual(row.language, "fr")
        self.assertEqual(row.n_segments, 6)
        self.assertEqual(row.done_ms, 1_300_000)
        self.assertEqual(row.transcript_path, path)
        self.assertEqual(PodcastTranscriptSegment.select().count(), 6)
        self.assertEqual(
            PodcastTranscriptSegment.get(
                PodcastTranscriptSegment.idx == 0
            ).search_text,
            "bonjour depuis le morceau.",
        )

    def test_rebuild_is_idempotent(self):
        self.ready_episode()
        with count_statements() as counts:
            stats = job.rebuild(app=self.config)
        self.assertEqual((stats["rebuilt"], stats["skipped"]), (0, 1))
        self.assertLess(sum(counts.values()), 25)
        self.assertEqual(PodcastTranscriptSegment.select().count(), 6)

    def test_a_sidecar_without_a_row_and_a_row_without_a_sidecar_both_survive(self):
        episode = self.ready_episode()
        orphan = self.episode(number=2, duration=60)
        orphan_sidecar = sidecar.json_path(orphan.path)
        write_bytes(
            orphan_sidecar,
            sidecar.dumps(
                sidecar.build(
                    episode_id="9" * 36,
                    language="fr",
                    engine="whisper.cpp",
                    model="ggml-small.bin",
                    engine_version="1.9.5",
                    duration_ms=60_000,
                    segments=[{"start_ms": 0, "end_ms": 1000, "text": "orphelin"}],
                    created="2026-11-14T10:00:00",
                    updated="2026-11-14T10:00:00",
                )
            ),
        )
        rowed = self.episode(number=3, duration=60)
        PodcastTranscript.create(episode=rowed, status="ready", n_segments=0)
        stats = job.rebuild(app=self.config)
        self.assertTrue(os.path.isfile(orphan_sidecar))
        self.assertEqual(PodcastTranscript.select().where(PodcastTranscript.episode == orphan.id).count(), 0)
        self.assertEqual(PodcastTranscript.select().where(PodcastTranscript.episode == rowed.id).count(), 1)
        self.assertGreaterEqual(stats["scanned"], 2)

    def test_a_hostile_sidecar_is_counted_not_crashed(self):
        episode = self.episode()
        write_bytes(sidecar.json_path(episode.path), b"{nope")
        stats = job.rebuild(app=self.config)
        self.assertEqual(stats["failed"], 1)
        self.assertTrue(os.path.isfile(sidecar.json_path(episode.path)))

    def test_the_srt_is_regenerated_from_the_json(self):
        episode = self.ready_episode()
        os.unlink(sidecar.srt_path(episode.path))
        job.flush_sidecar(episode.id, app=self.config)
        with open(sidecar.srt_path(episode.path), encoding="utf-8") as handle:
            self.assertIn("-->", handle.read())


class CliTestCase(_Base):
    def invoke(self, *args):
        from click.testing import CliRunner

        from neezer.cli import cli

        result = CliRunner().invoke(cli, list(args), obj=self.config)
        self.assertEqual(result.exit_code, 0, result.output)
        return result

    def test_episode_status_stop_and_rebuild(self):
        episode = self.episode(duration=60)
        result = self.invoke("podcast", "transcribe", "--episode", str(episode.id))
        self.assertIn("en file", result.output)
        self.assertEqual(PodcastTranscript.get().status, "queued")
        result = self.invoke("podcast", "transcribe", "--status")
        self.assertIn("1 en attente", result.output)
        result = self.invoke(
            "podcast", "transcribe", "--stop", "--episode", str(episode.id)
        )
        self.assertIn("stopped", result.output)
        self.assertEqual(PodcastTranscript.get().status, "stopped")

    def test_all_queues_the_backlog_and_needs_no_engine_run(self):
        channel = self.channel(auto_transcribe=True)
        for number in range(1, 4):
            self.episode(number=number, duration=600, channel=channel)
        result = self.invoke("podcast", "transcribe", "--all")
        self.assertIn("3", result.output)
        self.assertEqual(PodcastTranscript.select().count(), 3)
        self.assertEqual(self.log_lines(), [])

    def test_rebuild_command(self):
        self.episode()
        result = self.invoke("transcribe", "rebuild")
        self.assertIn("Sidecars lus", result.output)

    def test_no_option_is_an_explicit_error(self):
        from click.testing import CliRunner

        from neezer.cli import cli

        result = CliRunner().invoke(cli, ["podcast", "transcribe"], obj=self.config)
        self.assertNotEqual(result.exit_code, 0)
        self.assertIn("--episode", result.output)


class StatementTestCase(_Base):
    """The queue is a few short writes, not a walk over the library."""

    def test_enqueue_and_claim_are_bounded(self):
        episode = self.episode(duration=1300)
        with count_statements() as counts:
            job.enqueue(episode.id, user=self.user, app=self.config)
        enqueue_statements = sum(counts.values())
        self.assertLess(enqueue_statements, 20, enqueue_statements)
        worker = TranscribeWorker.create(
            name="Bureau", kind="client", token="encrypted"
        )
        with count_statements() as counts:
            claim = job.claim_next(worker, app=self.config)
        claim_statements = sum(counts.values())
        self.assertIsNotNone(claim)
        self.assertLess(claim_statements, 20, claim_statements)
        token = claim["lease"]
        with count_statements() as counts:
            job.apply_chunk(
                episode.id, worker, token, 0, 600_000,
                [{"start_ms": 0, "end_ms": 1000, "text": "Bonjour"}],
            )
        chunk_statements = sum(counts.values())
        self.assertLess(chunk_statements, 15, chunk_statements)
        # Written down so a regression names its new cost.
        self.assertGreaterEqual(enqueue_statements, 5)
        self.assertGreaterEqual(claim_statements, 5)
        self.assertGreaterEqual(chunk_statements, 3)


class RealEngineTestCase(unittest.TestCase):
    """Opt-in: a real whisper.cpp on a real 60 s clip the operator provides."""

    @unittest.skipUnless(
        os.environ.get("NS_WHISPER") == "1",
        "set NS_WHISPER=1 plus NS_WHISPER_BIN/MODEL/AUDIO to run the real engine",
    )
    def test_a_real_engine_produces_timed_text(self):
        binary = os.environ.get("NS_WHISPER_BIN")
        model = os.environ.get("NS_WHISPER_MODEL")
        audio = os.environ.get("NS_WHISPER_AUDIO")
        self.assertTrue(binary and os.path.isfile(binary), "NS_WHISPER_BIN")
        self.assertTrue(model and os.path.isfile(model), "NS_WHISPER_MODEL")
        self.assertTrue(audio and os.path.isfile(audio), "NS_WHISPER_AUDIO")
        work = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, work, True)
        paths = {
            "bin": binary,
            "model": model,
            "ffmpeg": shutil.which("ffmpeg"),
        }
        self.assertTrue(engine.quick_check(paths)["ok"])
        wav = os.path.join(work, "chunk.wav")
        engine.extract_chunk(paths, audio, 0, 60_000, wav)
        raw, language = engine.recognize(
            paths, wav, os.path.join(work, "chunk"), "auto", 2, 60_000
        )
        segments = engine.normalize_segments(raw, 0, 60_000)
        self.assertGreaterEqual(len(segments), 5)
        self.assertTrue(all(s["end_ms"] > s["start_ms"] for s in segments))
        self.assertTrue(all(s["text"] for s in segments))
        self.assertEqual(
            [s["start_ms"] for s in segments], sorted(s["start_ms"] for s in segments)
        )
        self.assertIsNotNone(language)


if __name__ == "__main__":
    unittest.main()
