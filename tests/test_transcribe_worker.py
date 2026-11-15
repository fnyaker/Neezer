# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""The transcription worker protocol, driven through the real Flask app.

The queue, the database, the lease and the archive are real; only the engine is
absent, because these routes never run one. A worker enrols with an admin
token, claims a queued episode, downloads its audio, posts timed chunks and
finishes. The tests pin the credential (shown once, encrypted at rest, never in
a log), the lease (one winner, a revoked worker is refused), the sidecar (always
off the request thread), the audio (Range, lease required) and the remote/locale
guards. No provider is touched anywhere in the file.
"""

import contextlib
import hashlib
import logging
import os
import shutil
import tempfile
import threading
import time
import unittest
import uuid
from datetime import datetime, timedelta
from unittest import mock

import peewee

from neezer import config as config_module
from neezer.config import DefaultConfig
from neezer.db import (
    PodcastChannel,
    PodcastEpisode,
    PodcastTranscript,
    PodcastTranscriptLease,
    PodcastTranscriptSegment,
    TranscribeWorker,
    db,
    release_database,
)
from neezer.deezer import workload
from neezer.deezer.provider import DeezerProvider
from neezer.managers.user import UserManager, decrypt_password
from neezer.transcribe import engine, job, sidecar
from neezer.web import create_application
from neezer.webui import remote
from neezer.webui.transcribe_worker import (
    LIMITERS,
    WORKER_POLICY,
    claim_limiter,
    create_limiter,
    transcribe_worker_api,
)


def write_file(path, data):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as handle:
        handle.write(data)
    return path


def wait_until(predicate, what, timeout=10.0):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(0.02)
    raise AssertionError("timed out waiting for " + what)


@contextlib.contextmanager
def statements():
    rows = []
    original = peewee.Database.execute_sql
    mine = threading.get_ident()

    def execute_sql(self, sql, *args, **kwargs):
        if threading.get_ident() == mine:
            rows.append(sql)
        return original(self, sql, *args, **kwargs)

    peewee.Database.execute_sql = execute_sql
    try:
        yield rows
    finally:
        peewee.Database.execute_sql = original


class WorkerConfig(DefaultConfig):
    TESTING = True

    def __init__(self, db_uri, archive, cache):
        super().__init__()
        self.BASE = dict(self.BASE, database_uri=db_uri)
        self.WEBAPP = dict(
            self.WEBAPP, cache_dir=cache, mount_webui=True, mount_api=True
        )
        self.DEEZER = dict(self.DEEZER, archive_dir=archive)
        self.TRANSCRIBE = dict(
            DefaultConfig.TRANSCRIBE,
            enabled=True,
            runner="external",
            language="fr",
            max_minutes=360,
        )


class TranscribeWorkerTestCase(unittest.TestCase):
    def setUp(self):
        self.policy = dict(remote.POLICY)
        self.addCleanup(self._restore_policy)
        self.tmp = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.archive = os.path.join(self.tmp, "archive")
        self.podcasts = os.path.join(self.archive, "Podcasts")
        self.cache = os.path.join(self.tmp, "cache")
        os.makedirs(self.podcasts)
        os.makedirs(self.cache)
        self.config = WorkerConfig(
            "sqlite:///" + os.path.join(self.tmp, "neezer.db"),
            self.archive,
            self.cache,
        )
        previous = config_module.current_config
        config_module.current_config = self.config
        self.addCleanup(setattr, config_module, "current_config", previous)
        self.app = create_application(self.config)
        if "transcribe_worker_api" not in self.app.blueprints:
            self.app.register_blueprint(transcribe_worker_api, url_prefix="/api")
        self.addCleanup(release_database)
        workload.reset_streams()
        self.addCleanup(workload.reset_streams)
        for limiter in LIMITERS:
            limiter.clear()
            self.addCleanup(limiter.clear)
        self.alice = UserManager.add("alice", "Alic3", admin=True)
        self.bob = UserManager.add("bob", "B0bbb", admin=False)
        self.client = self._login("alice", "Alic3")
        self.user_client = self._login("bob", "B0bbb")

    def _restore_policy(self):
        remote.POLICY.clear()
        remote.POLICY.update(self.policy)

    def _login(self, name, password):
        client = self.app.test_client()
        response = client.post(
            "/api/login", json={"username": name, "password": password}
        )
        self.assertEqual(response.status_code, 200, response.get_json())
        return client

    def _delete(self, path):
        try:
            os.unlink(path)
        except OSError:
            pass

    def channel(self, title="Show"):
        return PodcastChannel.create(
            user=self.alice, url="https://feed.example/" + title, title=title
        )

    def episode(self, channel=None, number=1, duration=600, archived=True):
        channel = channel or self.channel()
        path = None
        if archived:
            path = os.path.join(
                self.podcasts,
                channel.title,
                "2026-01-%02d - Episode %d.mp3" % (number, number),
            )
            write_file(path, b"\xff\xfb\x90\x00" * 256)
        return PodcastEpisode.create(
            channel=channel,
            title="Episode %d" % number,
            duration=duration,
            publish_date=datetime(2026, 1, 1) + timedelta(days=number),
            path=path,
            status="completed" if archived else "new",
        )

    def enqueue(self, episode):
        return job.enqueue(episode.id, app=self.config)

    def make_worker(self, name="Bureau", kind="client"):
        response = self.client.post(
            "/api/transcribe/workers", json={"name": name, "kind": kind}
        )
        self.assertEqual(response.status_code, 201, response.get_json())
        body = response.get_json()
        return body["worker"], body["token"]

    def headers(self, token, lease=None):
        out = {"Authorization": "Bearer " + token}
        if lease is not None:
            out["X-NS-Lease"] = lease
        return out

    def claim(self, token, client=None):
        return (client or self.app.test_client()).post(
            "/api/transcribe/worker/claim", headers=self.headers(token)
        )

    def claimed(self, token):
        response = self.claim(token)
        self.assertEqual(response.status_code, 200, response.get_json())
        return response.get_json()

    def chunk(self, token, lease, start, length, segments, **extra):
        body = {
            "start_ms": start,
            "duration_ms": length,
            "segments": segments,
            **extra,
        }
        return self.app.test_client().post(
            "/api/transcribe/worker/%s/chunk" % self.episode_id(lease),
            json=body,
            headers=self.headers(token, lease),
        )

    def episode_id(self, lease):
        row = PodcastTranscriptLease.get()
        return str(row.episode_id)

    def audio(self, token, lease, episode_id, headers=None):
        merged = self.headers(token, lease)
        merged.update(headers or {})
        return self.app.test_client().get(
            "/api/transcribe/worker/audio/" + str(episode_id), headers=merged
        )

    def test_admin_creates_a_worker_and_shows_the_token_once(self):
        worker, token = self.make_worker()
        self.assertEqual(worker["name"], "Bureau")
        self.assertEqual(worker["kind"], "client")
        self.assertTrue(worker["created"])
        self.assertTrue(token.startswith(worker["id"] + "."))
        self.assertGreater(len(token.split(".", 1)[1]), 20)

        row = TranscribeWorker.get()
        self.assertNotEqual(row.token, token)
        self.assertTrue(row.token.startswith("gcm:"))
        self.assertNotIn(token, row.token)
        self.assertEqual(decrypt_password(row.token), token)

        response = self.client.get("/api/transcribe/workers")
        self.assertEqual(response.status_code, 200)
        body = response.get_json()
        self.assertEqual(len(body["workers"]), 1)
        listed = body["workers"][0]
        self.assertEqual(
            sorted(listed),
            ["created", "enabled", "id", "kind", "last_seen", "lease", "name", "revoked"],
        )
        self.assertNotIn("token", listed)
        self.assertNotIn(token, response.get_data(as_text=True))
        self.assertEqual(response.headers["Cache-Control"], "no-store")

    def test_admin_listing_never_returns_a_token(self):
        first, first_token = self.make_worker("Bureau", "client")
        second, second_token = self.make_worker("Conteneur", "server")
        response = self.client.get("/api/transcribe/workers")
        text = response.get_data(as_text=True)
        self.assertNotIn(first_token, text)
        self.assertNotIn(second_token, text)
        kinds = {row["name"]: row["kind"] for row in response.get_json()["workers"]}
        self.assertEqual(kinds, {"Bureau": "client", "Conteneur": "server"})
        self.assertEqual(
            {row["id"] for row in response.get_json()["workers"]},
            {first["id"], second["id"]},
        )

    def test_admin_creation_validates_name_and_kind(self):
        for body in (
            {},
            {"name": "", "kind": "client"},
            {"name": "x" * 65, "kind": "client"},
            {"name": "a\nb", "kind": "client"},
            {"name": "Bureau", "kind": "phone"},
            {"name": 12, "kind": "client"},
        ):
            response = self.client.post("/api/transcribe/workers", json=body)
            self.assertEqual(response.status_code, 400, body)
        self.assertEqual(TranscribeWorker.select().count(), 0)

    def test_admin_routes_are_admin_only(self):
        worker, _token = self.make_worker()
        anonymous = self.app.test_client()
        self.assertEqual(anonymous.get("/api/transcribe/workers").status_code, 401)
        self.assertEqual(self.user_client.get("/api/transcribe/workers").status_code, 403)
        self.assertEqual(
            self.user_client.post(
                "/api/transcribe/workers", json={"name": "X", "kind": "client"}
            ).status_code,
            403,
        )
        self.assertEqual(
            self.user_client.delete(
                "/api/transcribe/workers/" + worker["id"]
            ).status_code,
            403,
        )
        self.assertEqual(
            self.client.delete("/api/transcribe/workers/not-a-uuid").status_code, 404
        )
        self.assertEqual(
            self.client.delete(
                "/api/transcribe/workers/00000000-0000-0000-0000-000000000000"
            ).status_code,
            404,
        )

    def test_a_cross_site_creation_is_refused_and_creates_nothing(self):
        response = self.client.post(
            "/api/transcribe/workers",
            json={"name": "Bureau", "kind": "client"},
            headers={"Origin": "https://evil.example"},
        )
        self.assertEqual(response.status_code, 403)
        self.assertEqual(TranscribeWorker.select().count(), 0)

    def test_admin_creation_is_rate_limited(self):
        with mock.patch.object(create_limiter, "max_events", 2):
            self.make_worker("Un", "client")
            self.make_worker("Deux", "client")
            response = self.client.post(
                "/api/transcribe/workers", json={"name": "Trois", "kind": "client"}
            )
        self.assertEqual(response.status_code, 429)
        self.assertGreaterEqual(int(response.headers["Retry-After"]), 1)
        self.assertEqual(response.get_json()["code"], "rate_limited")
        self.assertEqual(TranscribeWorker.select().count(), 2)

    def test_a_revoked_worker_is_refused_and_its_lease_is_freed(self):
        episode = self.episode()
        self.enqueue(episode)
        worker, token = self.make_worker()
        lease = self.claimed(token)["lease"]
        self.assertEqual(PodcastTranscript.get().status, "leased")
        revoked = self.client.delete("/api/transcribe/workers/" + worker["id"])
        self.assertEqual(revoked.status_code, 200)
        self.assertEqual(revoked.get_json()["released"], 1)
        self.assertEqual(PodcastTranscriptLease.select().count(), 0)
        self.assertEqual(PodcastTranscript.get().status, "queued")

        response = self.claim(token)
        self.assertEqual(response.status_code, 403)
        self.assertEqual(response.get_json()["code"], "worker_revoked")
        beat = self.app.test_client().post(
            "/api/transcribe/worker/%s/heartbeat" % episode.id,
            json={},
            headers=self.headers(token, lease),
        )
        self.assertEqual(beat.status_code, 403)
        again = self.client.delete("/api/transcribe/workers/" + worker["id"])
        self.assertEqual(again.status_code, 200)
        self.assertFalse(again.get_json()["revoked"])

    def test_a_disabled_worker_is_refused(self):
        worker, token = self.make_worker()
        row = TranscribeWorker.get(TranscribeWorker.id == worker["id"])
        row.enabled = False
        row.save()
        response = self.claim(token)
        self.assertEqual(response.status_code, 403)
        self.assertEqual(response.get_json()["code"], "worker_revoked")

    def test_worker_authentication_rejects_every_bad_credential(self):
        worker, token = self.make_worker()
        bad = [
            {},
            {"Authorization": "Basic YWJj"},
            {"Authorization": "Bearer"},
            {"Authorization": "Bearer garbage"},
            {"Authorization": "Bearer " + worker["id"] + ".wrong"},
            {"Authorization": "Bearer " + str(uuid.uuid4()) + ".wrong"},
        ]
        for headers in bad:
            response = self.app.test_client().post(
                "/api/transcribe/worker/claim", headers=headers
            )
            self.assertEqual(response.status_code, 401, headers)
            self.assertEqual(response.headers["WWW-Authenticate"], "Bearer")
            self.assertNotIn(token, response.get_data(as_text=True))

    def test_the_token_never_reaches_the_logs(self):
        messages = []

        class Capture(logging.Handler):
            def emit(self, record):
                messages.append(record.getMessage())

        handler = Capture()
        root = logging.getLogger()
        root.addHandler(handler)
        self.addCleanup(root.removeHandler, handler)

        worker, token = self.make_worker()
        self.app.test_client().post(
            "/api/transcribe/worker/claim",
            headers={"Authorization": "Bearer " + worker["id"] + ".wrong"},
        )
        episode = self.episode()
        self.enqueue(episode)
        self.claimed(token)
        text = "\n".join(messages)
        self.assertNotIn(token, text)
        self.assertNotIn(worker["id"] + ".", text)

    def test_claim_returns_a_lease_and_204_when_nothing_waits(self):
        episode = self.episode(duration=600)
        self.enqueue(episode)
        _worker, token = self.make_worker()
        response = self.claim(token)
        self.assertEqual(response.status_code, 200, response.get_json())
        body = response.get_json()
        self.assertEqual(body["episode_id"], str(episode.id))
        self.assertEqual(body["audio_url"], "/api/transcribe/worker/audio/" + str(episode.id))
        self.assertEqual(body["duration_ms"], 600_000)
        self.assertEqual(body["language"], "fr")
        self.assertEqual(body["chunk_ms"], 600_000)
        self.assertEqual(body["overlap_ms"], 2000)
        self.assertEqual(body["max_minutes"], 360)
        self.assertEqual(body["start_ms"], 0)
        self.assertTrue(body["expires_at"])
        self.assertLess(len(response.data), 1024)
        lease = PodcastTranscriptLease.get()
        self.assertEqual(
            lease.lease_token_hash,
            hashlib.sha256(body["lease"].encode("utf-8")).hexdigest(),
        )
        self.assertNotEqual(lease.lease_token_hash, body["lease"])
        self.assertEqual(PodcastTranscript.get().status, "leased")
        listed = self.client.get("/api/transcribe/workers").get_json()["workers"][0]
        self.assertEqual(listed["lease"]["episode_id"], str(episode.id))
        self.assertTrue(listed["lease"]["expires_at"])
        for _ in range(2):
            self.assertEqual(self.claim(token).status_code, 204)

    def test_two_claimers_get_one_lease(self):
        episode = self.episode()
        self.enqueue(episode)
        _first, first_token = self.make_worker("Un", "client")
        _second, second_token = self.make_worker("Deux", "client")
        results = []
        barrier = threading.Barrier(2)

        def take(token):
            client = self.app.test_client()
            barrier.wait()
            try:
                results.append(self.claim(token, client).status_code)
            finally:
                db.close()

        threads = [
            threading.Thread(target=take, args=(first_token,)),
            threading.Thread(target=take, args=(second_token,)),
        ]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join(20)
        self.assertEqual(sorted(results), [200, 204])
        self.assertEqual(PodcastTranscriptLease.select().count(), 1)
        self.assertEqual(PodcastTranscript.get().status, "leased")

    def test_claim_is_rate_limited_per_worker(self):
        episode = self.episode()
        self.enqueue(episode)
        _worker, token = self.make_worker()
        with mock.patch.object(claim_limiter, "max_events", 1):
            self.assertEqual(self.claim(token).status_code, 200)
            response = self.claim(token)
        self.assertEqual(response.status_code, 429)
        self.assertEqual(response.get_json()["code"], "rate_limited")

    def test_heartbeat_reports_cancel_and_progress(self):
        episode = self.episode(duration=600)
        self.enqueue(episode)
        _worker, token = self.make_worker()
        lease = self.claimed(token)["lease"]
        response = self.app.test_client().post(
            "/api/transcribe/worker/%s/heartbeat" % episode.id,
            json={"chunk_pct": 50},
            headers=self.headers(token, lease),
        )
        self.assertEqual(response.status_code, 200, response.get_json())
        body = response.get_json()
        self.assertFalse(body["cancel"])
        self.assertFalse(body["pause"])
        self.assertEqual(body["done_ms"], 0)
        self.assertTrue(body["expires_at"])
        self.assertAlmostEqual(job.progress(episode.id), 0.5, places=2)

        job.stop(episode.id, app=self.config)
        body = self.app.test_client().post(
            "/api/transcribe/worker/%s/heartbeat" % episode.id,
            json={},
            headers=self.headers(token, lease),
        ).get_json()
        self.assertTrue(body["cancel"])

        bad = self.app.test_client().post(
            "/api/transcribe/worker/%s/heartbeat" % episode.id,
            json={"chunk_pct": "half"},
            headers=self.headers(token, lease),
        )
        self.assertEqual(bad.status_code, 400)
        wrong = self.app.test_client().post(
            "/api/transcribe/worker/%s/heartbeat" % episode.id,
            json={},
            headers=self.headers(token, "not-the-lease"),
        )
        self.assertEqual(wrong.status_code, 409)
        self.assertEqual(wrong.get_json()["code"], "lease_expired")

    def test_chunk_stores_lines_and_is_idempotent(self):
        episode = self.episode(duration=1200)
        self.enqueue(episode)
        _worker, token = self.make_worker()
        lease = self.claimed(token)["lease"]
        segments = [
            {"start_ms": 1000, "end_ms": 4000, "text": " Bonjour \x00 à tous. "},
            {"start_ms": 5000, "end_ms": 9000, "text": "[Musique]"},
            {"start_ms": 9500, "end_ms": 9500, "text": "invalide"},
            "not an object",
        ]
        response = self.chunk(
            token,
            lease,
            0,
            600_000,
            segments,
            language="fr",
            engine="whisper.cpp",
            model="ggml-small.bin",
            engine_version="1.9.5",
        )
        self.assertEqual(response.status_code, 200, response.get_json())
        body = response.get_json()
        self.assertTrue(body["applied"])
        self.assertEqual(body["done_ms"], 600_000)
        self.assertEqual(body["n_segments"], 1)
        row = PodcastTranscript.get()
        self.assertEqual(row.language, "fr")
        self.assertEqual(row.engine, "whisper.cpp")
        self.assertEqual(row.model, "ggml-small.bin")
        self.assertEqual(row.engine_version, "1.9.5")
        segment = PodcastTranscriptSegment.get()
        self.assertEqual(segment.text, "Bonjour à tous.")
        self.assertEqual(segment.search_text, "bonjour a tous.")
        again = self.chunk(token, lease, 0, 600_000, segments)
        self.assertFalse(again.get_json()["applied"])
        self.assertEqual(again.get_json()["n_segments"], 1)
        self.assertEqual(PodcastTranscriptSegment.select().count(), 1)
        out_of_order = self.chunk(token, lease, 900_000, 600_000, [])
        self.assertEqual(out_of_order.status_code, 409)
        self.assertEqual(out_of_order.get_json()["code"], "chunk_out_of_order")
        missing = self.app.test_client().post(
            "/api/transcribe/worker/%s/chunk" % episode.id,
            json={"start_ms": 600_000, "duration_ms": 1000},
            headers=self.headers(token, lease),
        )
        self.assertEqual(missing.status_code, 400)

    def test_chunk_rejects_too_many_segments(self):
        episode = self.episode(duration=600)
        self.enqueue(episode)
        _worker, token = self.make_worker()
        lease = self.claimed(token)["lease"]
        response = self.chunk(
            token, lease, 0, 600_000, [{}] * (engine.MAX_SEGMENTS_PER_CHUNK + 1)
        )
        self.assertEqual(response.status_code, 409)
        self.assertEqual(response.get_json()["code"], "too_many_segments")
        self.assertEqual(PodcastTranscript.get().done_ms, 0)

    def test_chunk_body_over_one_mib_is_refused(self):
        episode = self.episode(duration=600)
        self.enqueue(episode)
        _worker, token = self.make_worker()
        lease = self.claimed(token)["lease"]
        response = self.app.test_client().post(
            "/api/transcribe/worker/%s/chunk" % episode.id,
            data=b"{" + b"x" * (1024 * 1024 + 100),
            content_type="application/json",
            headers=self.headers(token, lease),
        )
        self.assertEqual(response.status_code, 413)
        self.assertEqual(response.get_json()["code"], "too_large")

    def test_chunk_and_complete_flush_the_sidecar_off_the_request(self):
        episode = self.episode(duration=600)
        self.enqueue(episode)
        _worker, token = self.make_worker()
        lease = self.claimed(token)["lease"]
        entered = threading.Event()
        release = threading.Event()
        calls = []

        def fake_write(audio_path, payload, root):
            calls.append(threading.get_ident())
            entered.set()
            release.wait(10)
            return os.path.join(os.path.dirname(audio_path), "fake.transcript.json")

        with mock.patch.object(sidecar, "write", fake_write):
            response = self.chunk(
                token,
                lease,
                0,
                600_000,
                [{"start_ms": 1000, "end_ms": 4000, "text": "Bonjour."}],
            )
            self.assertEqual(response.status_code, 200, response.get_json())
            self.assertNotIn(threading.get_ident(), calls)
            self.assertTrue(entered.wait(5))
            self.assertFalse(release.is_set())
            self.assertEqual(PodcastTranscript.get().done_ms, 600_000)
            release.set()
            wait_until(
                lambda: PodcastTranscript.get().transcript_path is not None,
                "the sidecar path",
            )
            completed = self.app.test_client().post(
                "/api/transcribe/worker/%s/complete" % episode.id,
                json={"language": "fr"},
                headers=self.headers(token, lease),
            )
            self.assertEqual(completed.status_code, 200, completed.get_json())
            wait_until(lambda: len(calls) >= 2, "a second sidecar flush")
            self.assertNotIn(threading.get_ident(), calls)

    def test_a_late_flush_never_reverts_the_state(self):
        episode = self.episode(duration=600)
        self.enqueue(episode)
        _worker, token = self.make_worker()
        lease = self.claimed(token)["lease"]
        entered = threading.Event()
        release = threading.Event()
        calls = []

        def fake_write(audio_path, payload, root):
            calls.append(threading.get_ident())
            if len(calls) == 1:
                entered.set()
                release.wait(10)
            return os.path.join(os.path.dirname(audio_path), "fake.transcript.json")

        with mock.patch.object(sidecar, "write", fake_write):
            self.chunk(
                token,
                lease,
                0,
                600_000,
                [{"start_ms": 1000, "end_ms": 4000, "text": "Bonjour."}],
            )
            self.assertTrue(entered.wait(5))
            completed = self.app.test_client().post(
                "/api/transcribe/worker/%s/complete" % episode.id,
                json={"language": "fr", "model": "ggml-small.bin"},
                headers=self.headers(token, lease),
            )
            self.assertEqual(completed.status_code, 200, completed.get_json())
            self.assertEqual(PodcastTranscript.get().status, "ready")
            release.set()
            wait_until(lambda: len(calls) >= 2, "the second sidecar flush")
            wait_until(
                lambda: PodcastTranscript.get().status == "ready",
                "the ready state after both flushes",
            )

    def test_complete_requires_the_whole_audio_then_marks_ready(self):
        episode = self.episode(duration=600)
        self.enqueue(episode)
        worker, token = self.make_worker("Bureau", "client")
        lease = self.claimed(token)["lease"]
        early = self.app.test_client().post(
            "/api/transcribe/worker/%s/complete" % episode.id,
            json={"language": "fr"},
            headers=self.headers(token, lease),
        )
        self.assertEqual(early.status_code, 409)
        self.assertEqual(early.get_json()["code"], "incomplete")
        self.assertEqual(early.get_json()["done_ms"], 0)
        self.assertEqual(early.get_json()["duration_ms"], 600_000)
        self.chunk(
            token,
            lease,
            0,
            600_000,
            [{"start_ms": 1000, "end_ms": 4000, "text": "Bonjour."}],
        )
        response = self.app.test_client().post(
            "/api/transcribe/worker/%s/complete" % episode.id,
            json={
                "language": "fr",
                "engine": "whisper.cpp",
                "model": "ggml-small.bin",
                "engine_version": "1.9.5",
            },
            headers=self.headers(token, lease),
        )
        self.assertEqual(response.status_code, 200, response.get_json())
        self.assertEqual(response.get_json()["status"], "ready")
        row = PodcastTranscript.get()
        self.assertEqual(row.status, "ready")
        self.assertEqual(row.source, "client")
        self.assertEqual(row.worker_name, "Bureau")
        self.assertEqual(row.model, "ggml-small.bin")
        self.assertEqual(PodcastTranscriptLease.select().count(), 0)
        wait_until(lambda: PodcastTranscript.get().transcript_path, "the sidecar")
        self.assertTrue(os.path.isfile(PodcastTranscript.get().transcript_path))

    def test_complete_can_carry_the_final_chunk(self):
        episode = self.episode(duration=600)
        self.enqueue(episode)
        _worker, token = self.make_worker()
        lease = self.claimed(token)["lease"]
        response = self.app.test_client().post(
            "/api/transcribe/worker/%s/complete" % episode.id,
            json={
                "language": "fr",
                "segments": [
                    {"start_ms": 1000, "end_ms": 4000, "text": "Dernier mot."}
                ],
            },
            headers=self.headers(token, lease),
        )
        self.assertEqual(response.status_code, 200, response.get_json())
        self.assertEqual(response.get_json()["status"], "ready")
        self.assertEqual(PodcastTranscript.get().done_ms, 600_000)
        self.assertEqual(PodcastTranscriptSegment.select().count(), 1)

    def test_fail_bounds_and_cleans_the_cause(self):
        episode = self.episode(duration=600)
        self.enqueue(episode)
        _worker, token = self.make_worker()
        lease = self.claimed(token)["lease"]
        message = "  arrêt\x00 du moteur " + "x" * 500
        response = self.app.test_client().post(
            "/api/transcribe/worker/%s/fail" % episode.id,
            json={"code": "engine_failed", "message": message},
            headers=self.headers(token, lease),
        )
        self.assertEqual(response.status_code, 200, response.get_json())
        self.assertEqual(response.get_json()["status"], "error")
        row = PodcastTranscript.get()
        self.assertEqual(row.status, "error")
        self.assertEqual(row.error_code, "engine_failed")
        self.assertLessEqual(len(row.error_message), 300)
        self.assertNotIn("\x00", row.error_message)
        self.assertTrue(row.error_message.startswith("arrêt du moteur"))
        self.assertEqual(PodcastTranscriptLease.select().count(), 0)
        missing = self.app.test_client().post(
            "/api/transcribe/worker/%s/fail" % episode.id,
            json={"message": "no code"},
            headers=self.headers(token, lease),
        )
        self.assertEqual(missing.status_code, 400)

    def test_release_returns_the_episode_to_the_queue(self):
        episode = self.episode()
        self.enqueue(episode)
        _worker, token = self.make_worker()
        lease = self.claimed(token)["lease"]
        response = self.app.test_client().post(
            "/api/transcribe/worker/%s/release" % episode.id,
            json={},
            headers=self.headers(token, lease),
        )
        self.assertEqual(response.status_code, 200, response.get_json())
        self.assertEqual(response.get_json()["status"], "queued")
        self.assertEqual(PodcastTranscript.get().status, "queued")
        self.assertEqual(PodcastTranscriptLease.select().count(), 0)
        again = self.claimed(token)
        self.assertEqual(again["episode_id"], str(episode.id))
        self.assertEqual(again["start_ms"], 0)

    def test_audio_is_served_only_to_the_lease_holder(self):
        episode = self.episode()
        self.enqueue(episode)
        worker, token = self.make_worker("Bureau", "client")
        lease = self.claimed(token)["lease"]
        response = self.audio(token, lease, episode.id)
        self.assertEqual(response.status_code, 200)
        self.assertEqual(response.data, b"\xff\xfb\x90\x00" * 256)
        self.assertTrue(response.headers["Content-Type"].startswith("audio/"))
        self.assertEqual(response.headers["Cache-Control"], "no-store")
        self.assertNotIn(self.podcasts, response.headers.get("Content-Disposition", ""))
        self.assertNotIn(self.podcasts, response.data.decode("latin-1"))
        ranged = self.audio(token, lease, episode.id, {"Range": "bytes=0-99"})
        self.assertEqual(ranged.status_code, 206)
        self.assertEqual(len(ranged.data), 100)
        self.assertTrue(ranged.headers["Content-Range"].startswith("bytes 0-99/"))
        self.assertEqual(ranged.data, response.data[:100])
        ranged.close()
        response.close()

        self.assertEqual(self.audio(token, None, episode.id).status_code, 403)
        self.assertEqual(self.audio(token, "wrong", episode.id).status_code, 403)

        _other, other_token = self.make_worker("Autre", "client")
        self.assertEqual(self.audio(other_token, lease, episode.id).status_code, 403)
        self.assertEqual(
            self.audio(token, lease, "00000000-0000-0000-0000-000000000000").status_code,
            404,
        )
        self.assertEqual(self.audio(token, lease, "not-a-uuid").status_code, 404)

        unclaimed = self.episode(number=2)
        self.assertEqual(self.audio(token, lease, unclaimed.id).status_code, 404)
        self._delete(episode.path)
        self.assertEqual(self.audio(token, lease, episode.id).status_code, 404)
        self.assertEqual(worker["kind"], "client")

    def test_audio_of_a_revoked_worker_is_refused(self):
        episode = self.episode()
        self.enqueue(episode)
        worker, token = self.make_worker()
        lease = self.claimed(token)["lease"]
        self.client.delete("/api/transcribe/workers/" + worker["id"])
        self.assertEqual(self.audio(token, lease, episode.id).status_code, 403)

    def test_no_worker_payload_names_a_path_or_a_token(self):
        episode = self.episode()
        self.enqueue(episode)
        worker, token = self.make_worker()
        claim = self.claimed(token)
        lease = claim["lease"]
        payloads = [
            self.app.test_client().get("/api/transcribe/workers").get_data(as_text=True),
            self.app.test_client().post(
                "/api/transcribe/worker/claim",
                headers=self.headers(token),
            ).get_data(as_text=True),
            self.app.test_client().post(
                "/api/transcribe/worker/%s/heartbeat" % episode.id,
                json={},
                headers=self.headers(token, "wrong"),
            ).get_data(as_text=True),
            self.audio(token, "wrong", episode.id).get_data(as_text=True),
        ]
        for text in payloads:
            self.assertNotIn(self.archive, text)
            self.assertNotIn(episode.path, text)
            self.assertNotIn(token, text)
            self.assertNotIn("transcript_path", text)
            self.assertNotIn(worker["id"] + ".", text)
        self.assertNotIn("transcript_path", claim)

    def test_admin_listing_costs_three_statements(self):
        self.make_worker()
        with statements() as rows:
            response = self.client.get("/api/transcribe/workers")
        self.assertEqual(response.status_code, 200)
        self.assertEqual(len(rows), 3, rows)

    def test_claim_costs_a_bounded_number_of_statements(self):
        episode = self.episode()
        self.enqueue(episode)
        _worker, token = self.make_worker()
        client = self.app.test_client()
        with statements() as rows:
            response = client.post(
                "/api/transcribe/worker/claim", headers=self.headers(token)
            )
        self.assertEqual(response.status_code, 200, response.get_json())
        self.assertEqual(len(rows), 11, rows)

    def test_chunk_costs_a_bounded_number_of_statements(self):
        episode = self.episode(duration=600)
        self.enqueue(episode)
        _worker, token = self.make_worker()
        lease = self.claimed(token)["lease"]
        with statements() as rows:
            response = self.chunk(
                token,
                lease,
                0,
                600_000,
                [{"start_ms": 1000, "end_ms": 4000, "text": "Bonjour."}],
            )
        self.assertEqual(response.status_code, 200, response.get_json())
        self.assertEqual(len(rows), 8, rows)
        response.close()

    def test_audio_costs_a_bounded_number_of_statements(self):
        episode = self.episode()
        self.enqueue(episode)
        _worker, token = self.make_worker()
        lease = self.claimed(token)["lease"]
        with statements() as rows:
            response = self.audio(token, lease, episode.id)
        self.assertEqual(response.status_code, 200)
        self.assertEqual(len(rows), 6, rows)
        response.close()

    def test_remote_sessions_never_reach_a_worker_route(self):
        episode = self.episode()
        self.enqueue(episode)
        _worker, token = self.make_worker()
        lease = self.claimed(token)["lease"]
        link = self.client.post(
            "/api/remote/links", json={"level": "admin", "device": "dev12345"}
        ).get_json()["link"]
        remote_client = self.app.test_client()
        claimed = remote_client.post(
            "/api/remote/claim", json={"token": link["token"]}
        )
        self.assertEqual(claimed.status_code, 200, claimed.get_json())
        calls = [
            ("POST", "/api/transcribe/worker/claim", None),
            ("POST", "/api/transcribe/worker/%s/heartbeat" % episode.id, {}),
            (
                "POST",
                "/api/transcribe/worker/%s/chunk" % episode.id,
                {"start_ms": 0, "duration_ms": 1000, "segments": []},
            ),
            (
                "POST",
                "/api/transcribe/worker/%s/complete" % episode.id,
                {},
            ),
            (
                "POST",
                "/api/transcribe/worker/%s/fail" % episode.id,
                {"code": "x"},
            ),
            ("POST", "/api/transcribe/worker/%s/release" % episode.id, {}),
            ("GET", "/api/transcribe/worker/audio/%s" % episode.id, None),
        ]
        for method, path, body in calls:
            response = remote_client.open(
                path,
                method=method,
                json=body,
                headers=self.headers(token, lease),
            )
            self.assertEqual(response.status_code, 403, (method, path))
            self.assertEqual(response.get_json(), {"error": "not allowed", "level": "admin"})
        listed = remote_client.get("/api/transcribe/workers")
        self.assertEqual(listed.status_code, 200)

    def test_the_blueprint_registers_its_remote_policy(self):
        self.assertIn("transcribe_worker_api", self.app.blueprints)
        endpoints = {
            rule.endpoint.rsplit(".", 1)[-1]
            for rule in self.app.url_map.iter_rules()
            if rule.rule.startswith("/api/transcribe/worker")
        }
        self.assertLessEqual(endpoints, set(remote.POLICY))
        for name in (
            "transcribe_worker_claim",
            "transcribe_worker_heartbeat",
            "transcribe_worker_chunk",
            "transcribe_worker_complete",
            "transcribe_worker_fail",
            "transcribe_worker_release",
            "transcribe_worker_audio",
        ):
            self.assertEqual(remote.POLICY[name], remote.NEVER, name)
        for name in (
            "transcribe_workers",
            "transcribe_worker_create",
            "transcribe_worker_revoke",
        ):
            self.assertEqual(remote.POLICY[name], "admin", name)
        self.assertEqual(
            remote.POLICY["transcribe_worker_claim"],
            WORKER_POLICY["transcribe_worker_claim"],
        )

    def test_the_worker_api_never_calls_a_provider(self):
        with mock.patch.object(
            DeezerProvider, "dz", new_callable=mock.PropertyMock
        ) as client:
            client.side_effect = AssertionError("a provider was called")
            episode = self.episode(duration=600)
            self.enqueue(episode)
            _worker, token = self.make_worker()
            claim = self.claimed(token)
            lease = claim["lease"]
            audio = self.audio(token, lease, episode.id)
            self.assertEqual(audio.status_code, 200)
            audio.close()
            self.assertEqual(
                self.chunk(
                    token,
                    lease,
                    0,
                    600_000,
                    [{"start_ms": 1000, "end_ms": 4000, "text": "Bonjour."}],
                ).status_code,
                200,
            )
            self.assertEqual(
                self.app.test_client().post(
                    "/api/transcribe/worker/%s/complete" % episode.id,
                    json={"language": "fr"},
                    headers=self.headers(token, lease),
                ).status_code,
                200,
            )
