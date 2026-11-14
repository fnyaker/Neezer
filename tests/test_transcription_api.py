# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""The transcription API: who may read, who may start, and what it costs.

The engine is the same fake whisper/ffmpeg pair the queue tests use: nothing
here needs whisper.cpp, a model or the network. The routes are driven through
the real Flask app, the real session and the real queue, so a statement budget
means what it says.
"""

import contextlib
import json
import os
import shutil
import sys
import tempfile
import threading
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
    PodcastTranscriptSegment,
    release_database,
)
from neezer.deezer import workload
from neezer.managers.user import UserManager
from neezer.transcribe import engine, job, runner, settings
from neezer.web import create_application
from neezer.webui.remote import POLICY
from neezer.webui.transcribe import (
    mark_limiter,
    panel_search_limiter,
    read_limiter,
    search_limiter,
    start_hour_limiter,
    start_limiter,
    transcribe_api,
)

FAKE_WHISPER = r'''#!{python}
import json, sys

args = sys.argv[1:]
if "--version" in args:
    print("whisper-cli 1.9.5")
    raise SystemExit(0)

out = args[args.index("-of") + 1] if "-of" in args else None
document = {{
    "transcription": [
        {{"offsets": {{"from": 0, "to": 4000}}, "text": "Bonjour à tous."}},
        {{"offsets": {{"from": 6000, "to": 9000}}, "text": "La mémoire de travail."}},
    ],
    "result": {{"language": "fr"}},
}}
with open(out + ".json", "w", encoding="utf-8") as handle:
    json.dump(document, handle)
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

LIMITERS = (
    mark_limiter,
    start_limiter,
    start_hour_limiter,
    search_limiter,
    panel_search_limiter,
    read_limiter,
)


def write_file(path, data, mode=None):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as handle:
        handle.write(data)
    if mode is not None:
        os.chmod(path, mode)
    return path


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


class ApiConfig(DefaultConfig):
    TESTING = True

    def __init__(self, db_uri, archive, cache, **transcribe):
        super().__init__()
        self.BASE = dict(self.BASE, database_uri=db_uri)
        self.WEBAPP = dict(
            self.WEBAPP, cache_dir=cache, mount_webui=True, mount_api=True
        )
        self.DEEZER = dict(self.DEEZER, archive_dir=archive)
        self.TRANSCRIBE = dict(DefaultConfig.TRANSCRIBE, **transcribe)


class TranscriptionApiTestCase(unittest.TestCase):
    def setUp(self):
        self.policy = dict(POLICY)
        self.addCleanup(self._restore_policy)
        self.tmp = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.archive = os.path.join(self.tmp, "archive")
        self.podcasts = os.path.join(self.archive, "Podcasts")
        self.cache = os.path.join(self.tmp, "cache")
        self.bin_dir = os.path.join(self.tmp, "bin")
        self.model_dir = os.path.join(self.tmp, "models")
        os.makedirs(self.podcasts)
        os.makedirs(self.cache)
        os.makedirs(self.bin_dir)
        os.makedirs(self.model_dir)
        self.whisper = write_file(
            os.path.join(self.bin_dir, "whisper-cli"),
            FAKE_WHISPER.format(python=sys.executable).encode(),
            0o755,
        )
        self.ffmpeg = write_file(
            os.path.join(self.bin_dir, "ffmpeg"),
            FAKE_FFMPEG.format(python=sys.executable).encode(),
            0o755,
        )
        self.model = write_file(
            os.path.join(self.model_dir, "ggml-small.bin"),
            b"ggml" + b"\0" * 1_100_000,
        )
        self.env = mock.patch.dict(
            os.environ, {"PATH": self.bin_dir + os.pathsep + os.environ.get("PATH", "")}
        )
        self.env.start()
        self.addCleanup(self.env.stop)
        engine.clear_probe_cache()
        self.addCleanup(engine.clear_probe_cache)
        self.config = ApiConfig(
            "sqlite:///" + os.path.join(self.tmp, "neezer.db"),
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
        self.app = create_application(self.config)
        if "transcribe_api" not in self.app.blueprints:
            self.app.register_blueprint(transcribe_api, url_prefix="/api")
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
        POLICY.clear()
        POLICY.update(self.policy)

    def _login(self, name, password):
        client = self.app.test_client()
        response = client.post(
            "/api/login", json={"username": name, "password": password}
        )
        self.assertEqual(response.status_code, 200, response.get_json())
        return client

    # -- fixtures ---------------------------------------------------------------

    def channel(self, title="Show", auto=False, language=None):
        return PodcastChannel.create(
            user=self.alice,
            url="https://feed.example/" + title,
            title=title,
            auto_transcribe=auto,
            transcribe_language=language,
        )

    def episode(self, channel, number=1, duration=300, archived=True, title=None):
        name = title or "Episode %d" % number
        path = None
        if archived:
            path = os.path.join(
                self.podcasts, channel.title or "Show", "2026-01-%02d - %s.mp3" % (number, name)
            )
            write_file(path, b"\xff\xfb\x90\x00" * 256)
        return PodcastEpisode.create(
            channel=channel,
            title=name,
            duration=duration,
            publish_date=datetime(2026, 1, 1) + timedelta(days=number),
            path=path,
            status="completed" if archived else "new",
        )

    def transcript(
        self,
        episode,
        status="ready",
        language="fr",
        segments=None,
        error_code=None,
        error_message=None,
        total_segments=None,
        user="alice",
    ):
        duration_ms = int(episode.duration or 0) * 1000
        segments = segments if segments is not None else [
            (0, 6000, "Bonjour à tous."),
            (6000, 12000, "La mémoire de travail."),
        ]
        count = len(segments) if total_segments is None else total_segments
        row = PodcastTranscript.create(
            episode=episode,
            status=status,
            language=language,
            engine="whisper.cpp",
            model="ggml-small.bin",
            engine_version="1.9.5",
            duration_ms=duration_ms,
            done_ms=duration_ms if status == "ready" else 0,
            n_segments=count,
            error_code=error_code,
            error_message=error_message,
            source="server",
            worker_name="inline",
            user=self.alice if user == "alice" else None,
            queued_at=datetime.now().replace(microsecond=0),
            updated=datetime.now().replace(microsecond=0),
        )
        for idx, (start, end, text) in enumerate(segments):
            PodcastTranscriptSegment.create(
                transcript=row,
                idx=idx,
                start_ms=start,
                end_ms=end,
                text=text,
                search_text=job.fold(text),
            )
        return row

    def long_transcript(self, episode, count=30, step=10_000):
        segments = []
        for index in range(count):
            text = "La mémoire de travail." if index == 18 else "Phrase %d." % index
            start = index * step
            segments.append((start, start + step - 1000, text))
        episode.duration = count * step // 1000
        episode.save()
        return self.transcript(episode, segments=segments)

    def payload_text(self, response):
        return json.dumps(response.get_json())

    # -- access -----------------------------------------------------------------

    def test_every_route_requires_a_session(self):
        channel = self.channel()
        episode = self.episode(channel)
        anonymous = self.app.test_client()
        calls = [
            ("GET", "/api/transcribe/status", None),
            ("GET", "/api/transcribe/settings", None),
            ("PUT", "/api/transcribe/settings", {"language": "fr"}),
            ("POST", "/api/podcast/%s/transcribe" % channel.id, {"auto": False}),
            ("POST", "/api/podcast/episode/%s/transcribe" % episode.id, {}),
            ("POST", "/api/podcast/episode/%s/transcribe/stop" % episode.id, None),
            ("DELETE", "/api/podcast/episode/%s/transcript" % episode.id, None),
            ("GET", "/api/podcast/episode/%s/transcript" % episode.id, None),
            ("GET", "/api/podcast/%s/transcript/search?q=memoire" % channel.id, None),
            ("GET", "/api/transcripts/search?q=memoire", None),
        ]
        for method, path, body in calls:
            response = anonymous.open(path, method=method, json=body)
            self.assertEqual(response.status_code, 401, (method, path))

    def test_a_non_admin_reads_the_text_and_changes_nothing(self):
        channel = self.channel()
        episode = self.episode(channel)
        self.transcript(episode)
        self.assertEqual(
            self.user_client.get("/api/transcribe/status").status_code, 200
        )
        page = self.user_client.get(
            "/api/podcast/episode/%s/transcript" % episode.id
        )
        self.assertEqual(page.status_code, 200, page.get_json())
        self.assertEqual(len(page.get_json()["segments"]), 2)
        self.assertEqual(
            self.user_client.get(
                "/api/podcast/%s/transcript/search?q=memoire" % channel.id
            ).status_code,
            200,
        )
        self.assertEqual(
            self.user_client.get("/api/transcripts/search?q=memoire").status_code,
            200,
        )
        for method, path, body in (
            ("GET", "/api/transcribe/settings", None),
            ("PUT", "/api/transcribe/settings", {"language": "fr"}),
            ("POST", "/api/podcast/%s/transcribe" % channel.id, {"auto": True}),
            ("POST", "/api/podcast/episode/%s/transcribe" % episode.id, {}),
            ("POST", "/api/podcast/episode/%s/transcribe/stop" % episode.id, None),
            ("DELETE", "/api/podcast/episode/%s/transcript" % episode.id, None),
        ):
            response = self.user_client.open(path, method=method, json=body)
            self.assertEqual(response.status_code, 403, (method, path))
        self.assertEqual(PodcastTranscript.select().count(), 1)
        self.assertEqual(PodcastTranscript.get().status, "ready")

    def test_a_cross_site_write_is_refused_and_changes_nothing(self):
        channel = self.channel()
        episode = self.episode(channel)
        headers = {"Origin": "https://evil.example"}
        for method, path, body in (
            ("PUT", "/api/transcribe/settings", {"language": "en"}),
            ("POST", "/api/podcast/%s/transcribe" % channel.id, {"auto": True}),
            ("POST", "/api/podcast/episode/%s/transcribe" % episode.id, {}),
            ("DELETE", "/api/podcast/episode/%s/transcript" % episode.id, None),
        ):
            response = self.client.open(path, method=method, json=body, headers=headers)
            self.assertEqual(response.status_code, 403, (method, path))
        channel = PodcastChannel.get(PodcastChannel.id == channel.id)
        self.assertFalse(channel.auto_transcribe)
        self.assertEqual(PodcastTranscript.select().count(), 0)
        self.assertEqual(settings.load(self.app)["language"], "fr")

    # -- status -----------------------------------------------------------------

    def test_status_is_public_and_never_names_a_path(self):
        body = self.client.get("/api/transcribe/status").get_json()
        self.assertTrue(body["enabled"])
        self.assertTrue(body["available"])
        self.assertIsNone(body["reason"])
        self.assertEqual(body["runner"], "inline")
        self.assertEqual(body["language"], "fr")
        self.assertEqual(body["max_minutes"], 360)
        self.assertEqual(body["daily_minutes"], 120)
        self.assertEqual(body["search_segments"], 1_200_000)
        self.assertEqual(body["model"], "ggml-small.bin")
        self.assertEqual(
            set(body["queue"]),
            {
                "queued",
                "leased",
                "running",
                "ready",
                "error",
                "stopped",
                "budget_used_ms",
                "budget_limit_ms",
            },
        )
        text = json.dumps(body)
        self.assertNotIn(self.model_dir, text)
        self.assertNotIn(self.bin_dir, text)
        self.assertNotIn("transcript_path", text)
        self.assertIsNone(body["last_error"])
        bob = self.user_client.get("/api/transcribe/status").get_json()
        self.assertNotIn("last_error", bob)
        self.assertTrue(bob["enabled"])
        self.assertNotIn(self.model_dir, json.dumps(bob))

    def test_status_counts_the_queue_and_says_the_last_failure(self):
        channel = self.channel()
        ready = self.episode(channel, number=1)
        self.transcript(ready)
        queued = self.episode(channel, number=2)
        PodcastTranscript.create(
            episode=queued,
            status="queued",
            duration_ms=600_000,
            queued_at=datetime.now().replace(microsecond=0),
            updated=datetime.now().replace(microsecond=0),
        )
        failed = self.episode(channel, number=3)
        self.transcript(
            failed,
            status="error",
            error_code="exit",
            error_message="le moteur s'est arrêté (%s/Podcasts/Show/x.mp3)" % self.archive,
        )
        admin = self.client.get("/api/transcribe/status").get_json()
        self.assertEqual(admin["queue"]["ready"], 1)
        self.assertEqual(admin["queue"]["queued"], 1)
        self.assertEqual(admin["queue"]["error"], 1)
        self.assertEqual(admin["queue"]["budget_used_ms"], 600_000)
        self.assertEqual(admin["queue"]["budget_limit_ms"], 120 * 60_000)
        self.assertEqual(admin["last_error"]["code"], "exit")
        self.assertNotIn(self.archive, admin["last_error"]["message"])
        bob = self.user_client.get("/api/transcribe/status").get_json()
        self.assertNotIn("last_error", bob)

    # -- settings ---------------------------------------------------------------

    def test_settings_are_admin_only_bounded_and_never_a_path(self):
        self.assertEqual(
            self.user_client.get("/api/transcribe/settings").status_code, 403
        )
        body = self.client.get("/api/transcribe/settings").get_json()
        self.assertEqual(
            body["languages"], ["auto", "fr", "en", "es", "de", "it", "pt"]
        )
        self.assertEqual(body["engine"]["name"], "whisper.cpp")
        self.assertEqual(body["model"]["name"], "ggml-small.bin")
        self.assertTrue(body["engine"]["ok"])
        self.assertNotIn(self.model_dir, json.dumps(body))
        self.assertNotIn("transcript_path", json.dumps(body))

        rejected = (
            ({"bin": self.whisper}, "paths_are_server_side"),
            ({"model": self.model}, "paths_are_server_side"),
            ({"vad_model": "/tmp/x"}, "paths_are_server_side"),
            ({"cpuset": "0-1"}, "paths_are_server_side"),
            ({"runner": "external"}, "server_side_setting"),
            ({"max_minutes": 4}, "max_minutes"),
            ({"max_minutes": 1441}, "max_minutes"),
            ({"daily_minutes": -1}, "daily_minutes"),
            ({"search_segments": -1}, "search_segments"),
            ({"language": "zz"}, "language"),
            ({"enabled": "yes"}, "enabled"),
            ({"unknown": 1}, "unknown_setting"),
            ({}, "invalid"),
        )
        for payload, code in rejected:
            response = self.client.put("/api/transcribe/settings", json=payload)
            self.assertEqual(response.status_code, 400, payload)
            self.assertEqual(response.get_json()["code"], code, payload)
        self.assertFalse(Meta.select().where(Meta.key.startswith("tr.")).exists())
        self.assertTrue(settings.load(self.app)["enabled"])

        response = self.client.put(
            "/api/transcribe/settings",
            json={"language": "en", "max_minutes": 60, "enabled": False},
        )
        self.assertEqual(response.status_code, 200, response.get_json())
        self.assertEqual(response.get_json()["language"], "en")
        self.assertFalse(settings.load(self.app)["enabled"])
        self.assertEqual(settings.load(self.app)["max_minutes"], 60)
        self.assertEqual(
            Meta.get(Meta.key == "tr.language").value, "en"
        )
        response = self.client.put(
            "/api/transcribe/settings", json={"daily_minutes": 0}
        )
        self.assertEqual(response.get_json()["daily_minutes"], 0)

    def test_enabling_without_a_usable_engine_is_refused_and_saves_nothing(self):
        self.app.config["TRANSCRIBE"]["enabled"] = False
        self.app.config["TRANSCRIBE"]["bin"] = None
        engine.clear_probe_cache()
        response = self.client.put(
            "/api/transcribe/settings", json={"enabled": True}
        )
        self.assertEqual(response.status_code, 409)
        body = response.get_json()
        self.assertEqual(body["code"], "unavailable")
        self.assertEqual(body["reason"], "engine_missing")
        self.assertFalse(Meta.select().where(Meta.key == "tr.enabled").exists())
        self.assertFalse(settings.load(self.app)["enabled"])

    def test_put_never_starts_a_job(self):
        with mock.patch.object(job, "enqueue") as enqueue, mock.patch.object(
            runner, "wake"
        ) as wake:
            response = self.client.put(
                "/api/transcribe/settings", json={"language": "fr"}
            )
        self.assertEqual(response.status_code, 200)
        enqueue.assert_not_called()
        wake.assert_not_called()

    def test_the_probe_reports_the_engine_version_without_a_path(self):
        body = self.client.get("/api/transcribe/settings?probe=1").get_json()
        self.assertEqual(body["engine"]["version"], "whisper-cli 1.9.5")
        self.assertTrue(body["engine"]["ok"])
        self.assertNotIn(self.bin_dir, json.dumps(body))

    # -- marking a show ---------------------------------------------------------

    def test_marking_a_show_queues_a_bounded_backlog_and_starts_no_engine(self):
        channel = self.channel()
        for number in range(1, 4):
            self.episode(channel, number=number, duration=20)
        with mock.patch.object(runner, "wake") as wake:
            response = self.client.post(
                "/api/podcast/%s/transcribe" % channel.id,
                json={"auto": True, "backlog": "recent", "language": "fr"},
            )
        self.assertEqual(response.status_code, 202, response.get_json())
        body = response.get_json()
        self.assertTrue(body["auto_transcribe"])
        self.assertEqual(body["transcribe_language"], "fr")
        self.assertEqual(body["backlog"], "recent")
        self.assertEqual(body["queued"], 3)
        self.assertEqual(body["estimate"]["episodes"], 3)
        self.assertEqual(body["estimate"]["audio_hours"], 0.0)
        self.assertEqual(PodcastTranscript.select().where(
            PodcastTranscript.status == "queued").count(), 3)
        wake.assert_called()

        second = self.client.post(
            "/api/podcast/%s/transcribe" % channel.id, json={"auto": True}
        )
        self.assertEqual(second.status_code, 429)
        self.assertIn("retry_in", second.get_json())
        self.assertIn("Retry-After", second.headers)
        self.assertEqual(PodcastTranscript.select().count(), 3)

        off = self.client.post(
            "/api/podcast/%s/transcribe" % channel.id,
            json={"auto": False},
        )
        self.assertEqual(off.status_code, 200)
        self.assertFalse(off.get_json()["auto_transcribe"])
        self.assertEqual(off.get_json()["backlog"], "recent")
        self.assertEqual(PodcastTranscript.select().count(), 3)
        self.assertEqual(
            Meta.get(Meta.key == "tr.bl." + str(channel.id)[:24]).value, "recent"
        )

        channel.auto_transcribe = True
        channel.save()
        for row in PodcastTranscript.select():
            row.delete_instance()
        mark_limiter.clear()
        none = self.client.post(
            "/api/podcast/%s/transcribe" % channel.id,
            json={"auto": True, "backlog": "none"},
        )
        self.assertEqual(none.status_code, 202)
        self.assertEqual(none.get_json()["queued"], 0)
        self.assertEqual(PodcastTranscript.select().count(), 0)

    def test_the_daily_budget_stops_the_backlog_and_keeps_episodes_none(self):
        self.app.config["TRANSCRIBE"]["daily_minutes"] = 1
        channel = self.channel()
        for number in range(1, 3):
            self.episode(channel, number=number, duration=1200)
        response = self.client.post(
            "/api/podcast/%s/transcribe" % channel.id,
            json={"auto": True, "backlog": "all"},
        )
        self.assertEqual(response.status_code, 202, response.get_json())
        self.assertEqual(response.get_json()["queued"], 0)
        self.assertEqual(response.get_json()["skipped"].get("budget"), 1)
        self.assertEqual(PodcastTranscript.select().count(), 0)
        self.assertFalse(
            PodcastTranscript.select()
            .where(PodcastTranscript.status == "queued")
            .exists()
        )

    def test_marking_validates_its_body_and_the_show(self):
        channel = self.channel()
        for payload in (
            {"auto": "yes"},
            {"auto": True, "backlog": "everything"},
            {"auto": True, "language": "zz"},
            {},
        ):
            response = self.client.post(
                "/api/podcast/%s/transcribe" % channel.id, json=payload
            )
            self.assertEqual(response.status_code, 400, payload)
        self.assertEqual(
            self.client.post(
                "/api/podcast/%s/transcribe" % "0" * 36, json={"auto": False}
            ).status_code,
            404,
        )
        self.assertEqual(
            self.client.post(
                "/api/podcast/not-an-id/transcribe", json={"auto": False}
            ).status_code,
            404,
        )

    # -- one episode ------------------------------------------------------------

    def test_start_stop_delete_and_a_real_job_on_the_fake_engine(self):
        channel = self.channel()
        episode = self.episode(channel, duration=20)
        started = self.client.post(
            "/api/podcast/episode/%s/transcribe" % episode.id, json={}
        )
        self.assertEqual(started.status_code, 202, started.get_json())
        self.assertEqual(started.get_json()["state"], "queued")
        self.assertEqual(PodcastTranscript.get().status, "queued")
        again = self.client.post(
            "/api/podcast/episode/%s/transcribe" % episode.id, json={}
        )
        self.assertEqual(again.status_code, 409)
        self.assertEqual(again.get_json()["code"], "already_running")
        stopped = self.client.post(
            "/api/podcast/episode/%s/transcribe/stop" % episode.id
        )
        self.assertEqual(stopped.status_code, 200)
        self.assertEqual(stopped.get_json()["status"], "stopped")
        self.assertEqual(PodcastTranscript.get().status, "stopped")
        second = self.client.post(
            "/api/podcast/episode/%s/transcribe/stop" % episode.id
        )
        self.assertEqual(second.status_code, 409)
        self.assertEqual(second.get_json()["code"], "not_running")
        resumed = self.client.post(
            "/api/podcast/episode/%s/transcribe" % episode.id, json={}
        )
        self.assertEqual(resumed.status_code, 202)
        self.assertEqual(PodcastTranscript.get().status, "queued")

        self.assertEqual(runner.drain(self.app, limit=1), 1)
        row = PodcastTranscript.get()
        self.assertEqual(row.status, "ready")
        self.assertEqual(row.n_segments, 2)
        body = self.client.get(
            "/api/podcast/episode/%s/transcript" % episode.id
        ).get_json()
        self.assertEqual(body["status"], "ready")
        self.assertEqual(body["progress"], 1.0)
        self.assertEqual(len(body["segments"]), 2)
        self.assertEqual(body["segments"][1]["text"], "La mémoire de travail.")
        self.assertNotIn("transcript_path", json.dumps(body))
        self.assertNotIn(self.archive, json.dumps(body))

        ready = self.client.post(
            "/api/podcast/episode/%s/transcribe" % episode.id, json={}
        )
        self.assertEqual(ready.status_code, 409)
        self.assertEqual(ready.get_json()["code"], "ready")
        forced = self.client.post(
            "/api/podcast/episode/%s/transcribe" % episode.id,
            json={"force": True, "language": "auto"},
        )
        self.assertEqual(forced.status_code, 202, forced.get_json())
        self.assertEqual(PodcastTranscriptSegment.select().count(), 0)
        self.assertEqual(
            self.client.post(
                "/api/podcast/episode/%s/transcribe/stop" % episode.id
            ).status_code,
            200,
        )

        self.assertEqual(
            self.client.delete(
                "/api/podcast/episode/%s/transcript" % episode.id
            ).status_code,
            204,
        )
        self.assertFalse(PodcastTranscript.select().exists())
        self.assertEqual(PodcastTranscriptSegment.select().count(), 0)
        self.assertTrue(os.path.isfile(episode.path))
        self.assertEqual(
            self.client.delete(
                "/api/podcast/episode/%s/transcript" % episode.id
            ).status_code,
            204,
        )

    def test_starting_an_unarchived_episode_says_why(self):
        channel = self.channel()
        episode = self.episode(channel, archived=False)
        response = self.client.post(
            "/api/podcast/episode/%s/transcribe" % episode.id, json={}
        )
        self.assertEqual(response.status_code, 409)
        self.assertEqual(response.get_json()["code"], "not_archived")
        self.assertFalse(PodcastTranscript.select().exists())

    def test_deleting_a_running_transcript_is_refused(self):
        channel = self.channel()
        episode = self.episode(channel)
        self.transcript(episode, status="queued")
        response = self.client.delete(
            "/api/podcast/episode/%s/transcript" % episode.id
        )
        self.assertEqual(response.status_code, 409)
        self.assertEqual(response.get_json()["code"], "running")
        self.assertTrue(PodcastTranscript.select().exists())

    def test_an_episode_route_404s_on_junk_and_unknowns(self):
        for value in ("junk", "0" * 36):
            for method, suffix in (
                ("POST", "/transcribe"),
                ("POST", "/transcribe/stop"),
                ("DELETE", "/transcript"),
                ("GET", "/transcript"),
            ):
                response = self.client.open(
                    "/api/podcast/episode/%s%s" % (value, suffix), method=method, json={}
                )
                self.assertEqual(response.status_code, 404, (value, suffix))

    def test_a_start_is_rate_limited_and_creates_nothing_when_refused(self):
        channel = self.channel()
        episodes = [self.episode(channel, number=n) for n in range(1, 4)]
        for _ in range(3):
            start_limiter.allow(str(self.alice.id))
        response = self.client.post(
            "/api/podcast/episode/%s/transcribe" % episodes[0].id, json={}
        )
        self.assertEqual(response.status_code, 429)
        self.assertIn("Retry-After", response.headers)
        self.assertFalse(PodcastTranscript.select().exists())

    # -- reading one episode ----------------------------------------------------

    def test_the_page_at_query_and_episode_search(self):
        channel = self.channel()
        episode = self.episode(channel)
        self.long_transcript(episode, count=30)
        first = self.client.get(
            "/api/podcast/episode/%s/transcript?limit=2" % episode.id
        ).get_json()
        self.assertEqual(len(first["segments"]), 2)
        self.assertEqual(first["total"], 30)
        self.assertEqual(first["offset"], 0)
        self.assertEqual(first["next"], 2)
        self.assertEqual(first["segments"][0]["start"], 0.0)
        self.assertEqual(first["segments"][0]["end"], 9.0)
        second = self.client.get(
            "/api/podcast/episode/%s/transcript?limit=2&offset=2" % episode.id
        ).get_json()
        self.assertEqual(second["offset"], 2)
        self.assertEqual(second["next"], 4)
        self.assertEqual(second["segments"][0]["start"], 20.0)

        around = self.client.get(
            "/api/podcast/episode/%s/transcript?limit=10&at=185" % episode.id
        ).get_json()
        self.assertEqual(around["offset"], 10)
        self.assertEqual(around["segments"][0]["start"], 100.0)

        found = self.client.get(
            "/api/podcast/episode/%s/transcript?q=memoire" % episode.id
        ).get_json()
        self.assertEqual(found["query"], "memoire")
        self.assertEqual(found["total"], 1)
        self.assertEqual(len(found["segments"]), 1)
        self.assertIn("mémoire", found["segments"][0]["text"])
        self.assertLessEqual(len(found["segments"][0]["text"]), 240)
        self.assertIsNone(found["next"])

        for query in ("a", "un deux trois quatre cinq six sept"):
            response = self.client.get(
                "/api/podcast/episode/%s/transcript?q=%s"
                % (episode.id, query.replace(" ", "%20"))
            )
            self.assertEqual(response.status_code, 400, query)
            self.assertEqual(response.get_json()["code"], "q")
        self.assertEqual(
            self.client.get(
                "/api/podcast/episode/%s/transcript?limit=0&offset=-3"
                % episode.id
            ).status_code,
            200,
        )

    def test_like_wildcards_in_a_query_match_literally(self):
        from urllib.parse import quote

        channel = self.channel()
        episode = self.episode(channel)
        self.transcript(
            episode,
            segments=[
                (0, 5000, "Le budget est de 100% cette année."),
                (5000, 10000, "Le code a_b reste stable."),
                (10000, 15000, "Le code axb reste stable."),
                (15000, 20000, "Un millier de choses."),
            ],
        )
        for query, expected in (("100%", "100%"), ("a_b", "a_b")):
            body = self.client.get(
                "/api/podcast/episode/%s/transcript?q=%s"
                % (episode.id, quote(query))
            ).get_json()
            self.assertEqual(body["total"], 1, query)
            self.assertIn(expected, body["segments"][0]["text"], query)

    def test_an_episode_without_a_transcript_is_a_200_none(self):
        channel = self.channel()
        episode = self.episode(channel)
        body = self.client.get(
            "/api/podcast/episode/%s/transcript" % episode.id
        ).get_json()
        self.assertEqual(body["status"], "none")
        self.assertEqual(body["segments"], [])
        self.assertEqual(body["total"], 0)
        self.assertTrue(body["archived"])
        unarchived = self.episode(channel, number=2, archived=False)
        body = self.client.get(
            "/api/podcast/episode/%s/transcript" % unarchived.id
        ).get_json()
        self.assertFalse(body["archived"])

    def test_an_error_cause_is_scrubbed_and_admin_only(self):
        channel = self.channel()
        episode = self.episode(channel)
        message = "le moteur s'est arrêté (%s/Podcasts/Show/x.mp3)" % self.archive
        self.transcript(
            episode, status="error", error_code="exit", error_message=message
        )
        bob = self.user_client.get(
            "/api/podcast/episode/%s/transcript" % episode.id
        ).get_json()
        self.assertEqual(bob["error"], {"code": "exit"})
        admin = self.client.get(
            "/api/podcast/episode/%s/transcript" % episode.id
        ).get_json()
        self.assertEqual(admin["error"]["code"], "exit")
        self.assertNotIn(self.archive, admin["error"]["message"])

    def test_a_read_is_rate_limited(self):
        channel = self.channel()
        episode = self.episode(channel)
        self.transcript(episode)
        for _ in range(120):
            read_limiter.allow(str(self.alice.id))
        response = self.client.get(
            "/api/podcast/episode/%s/transcript" % episode.id
        )
        self.assertEqual(response.status_code, 429)
        self.assertIn("Retry-After", response.headers)

    # -- searching a show and the library --------------------------------------

    def search_show(self, channel, query="memoire", limit=None):
        path = "/api/podcast/%s/transcript/search?q=%s" % (channel.id, query)
        if limit is not None:
            path += "&limit=%d" % limit
        return self.client.get(path)

    def test_a_show_search_returns_bounded_excerpts_and_a_scope(self):
        channel = self.channel()
        older = self.episode(channel, number=1)
        self.transcript(older)
        newer = self.episode(channel, number=2)
        self.transcript(
            newer,
            segments=[
                (0, 4000, "Une longue introduction sans le mot cherché."),
                (4000, 9000, "Ici la mémoire de travail revient, mémoire encore."),
            ],
        )
        broken = self.episode(channel, number=3)
        self.transcript(broken, status="error", error_code="exit")
        body = self.search_show(channel).get_json()
        self.assertEqual(body["q"], "memoire")
        self.assertEqual(body["total"], 2)
        self.assertFalse(body["more"])
        self.assertEqual(body["scope"], {"episodes": 2, "truncated": False})
        self.assertEqual([hit["episode_id"] for hit in body["hits"]], [str(newer.id), str(older.id)])
        first = body["hits"][0]
        self.assertEqual(
            set(first["episode"]), {"id", "title", "cover", "published"}
        )
        self.assertEqual(first["channel_title"], "Show")
        self.assertIn("mémoire", first["text"])
        self.assertLessEqual(len(first["text"]), 240)
        self.assertGreaterEqual(first["score"], 2)
        self.assertGreater(first["start"], 0)
        self.assertGreater(first["end"], first["start"])
        self.assertNotIn("transcript_path", json.dumps(body))

        empty = self.search_show(channel, "").get_json()
        self.assertEqual((empty["hits"], empty["total"]), ([], 0))
        self.assertEqual(self.search_show(channel, "m").status_code, 400)

    def test_a_show_search_scope_is_the_last_hundred_ready_episodes(self):
        channel = self.channel()
        for number in range(1, 102):
            episode = self.episode(channel, number=number, duration=20)
            self.transcript(
                episode,
                segments=[(0, 5000, "La mémoire de travail.")],
            )
        body = self.search_show(channel, limit=50).get_json()
        self.assertEqual(body["scope"]["episodes"], 100)
        self.assertTrue(body["scope"]["truncated"])
        self.assertEqual(body["total"], 50)
        self.assertTrue(body["more"])

    def test_the_library_search_says_its_scope_and_what_is_partial(self):
        first = self.channel(title="First")
        second = self.channel(title="Second")
        old = self.episode(first, number=1)
        self.transcript(old)
        new = self.episode(second, number=2)
        self.transcript(new)
        body = self.client.get("/api/transcripts/search?q=memoire").get_json()
        self.assertEqual(body["total"], 2)
        self.assertEqual(body["scope"]["episodes"], 2)
        self.assertEqual(body["scope"]["segments"], 4)
        self.assertFalse(body["scope"]["truncated"])
        self.assertEqual(body["scope"]["limit_segments"], 1_200_000)
        self.assertEqual(body["hits"][0]["episode_id"], str(new.id))
        self.assertEqual(body["hits"][0]["episode"]["title"], "Episode 2")

        limited = self.client.get("/api/transcripts/search?q=memoire&limit=1").get_json()
        self.assertEqual(len(limited["hits"]), 1)
        self.assertTrue(limited["more"])
        self.assertEqual(self.client.get(
            "/api/transcripts/search?q=memoire&limit=999"
        ).get_json()["more"], False)

        self.app.config["TRANSCRIBE"]["search_segments"] = 1
        partial = self.client.get("/api/transcripts/search?q=memoire").get_json()
        self.assertEqual(partial["scope"]["episodes"], 1)
        self.assertEqual(partial["scope"]["segments"], 2)
        self.assertTrue(partial["scope"]["truncated"])
        self.assertEqual(partial["scope"]["limit_segments"], 1)

        self.assertEqual(self.client.get("/api/transcripts/search").status_code, 200)

    def test_search_is_rate_limited(self):
        for _ in range(30):
            search_limiter.allow(str(self.alice.id))
        response = self.client.get("/api/transcripts/search?q=memoire")
        self.assertEqual(response.status_code, 429)

    # -- the show and episode payloads -----------------------------------------

    def test_the_show_payload_carries_the_transcription_state(self):
        channel = self.channel()
        ready = self.episode(channel, number=1)
        self.transcript(ready)
        queued = self.episode(channel, number=2)
        PodcastTranscript.create(
            episode=queued,
            status="queued",
            duration_ms=300_000,
            queued_at=datetime.now().replace(microsecond=0),
            updated=datetime.now().replace(microsecond=0),
        )
        body = self.client.get("/api/podcast/%s" % channel.id).get_json()
        self.assertFalse(body["auto_transcribe"])
        self.assertEqual(body["transcribe_language"], "auto")
        self.assertEqual(body["transcribed_count"], 1)
        by_id = {episode["id"]: episode for episode in body["episodes"]}
        self.assertEqual(by_id[str(ready.id)]["transcript"]["status"], "ready")
        self.assertEqual(by_id[str(ready.id)]["transcript"]["progress"], 1.0)
        self.assertEqual(by_id[str(ready.id)]["transcript"]["segment_count"], 2)
        self.assertEqual(by_id[str(queued.id)]["transcript"]["status"], "queued")
        self.assertEqual(by_id[str(queued.id)]["transcript"]["progress"], 0.0)
        self.assertNotIn("transcript_path", json.dumps(body))
        self.assertNotIn(self.archive, json.dumps(body))
        listed = self.client.get("/api/podcasts").get_json()["podcasts"][0]
        self.assertEqual(listed["auto_transcribe"], False)
        self.assertNotIn("episodes", listed)

    # -- cost and containment ---------------------------------------------------

    def test_one_page_is_three_statements_and_one_search_is_one_like(self):
        channel = self.channel()
        episode = self.episode(channel)
        self.long_transcript(episode, count=5)
        with statements() as sql:
            response = self.client.get(
                "/api/podcast/episode/%s/transcript?limit=2" % episode.id
            )
        self.assertEqual(response.status_code, 200)
        self.assertEqual(len(sql), 3, sql)
        with statements() as sql:
            self.assertEqual(self.search_show(channel).status_code, 200)
        self.assertEqual(len(sql), 4, sql)
        self.assertEqual(
            sum(
                1
                for text in sql
                if "search_text" in text and " LIKE " in text.upper()
            ),
            1,
            sql,
        )
        with statements() as sql:
            self.assertEqual(
                self.client.get("/api/transcripts/search?q=memoire").status_code,
                200,
            )
        self.assertEqual(len(sql), 4, sql)
        self.assertEqual(
            sum(
                1
                for text in sql
                if "search_text" in text and " LIKE " in text.upper()
            ),
            1,
            sql,
        )

    def test_the_show_page_adds_one_grouped_read(self):
        channel = self.channel()
        first = self.episode(channel, number=1)
        second = self.episode(channel, number=2)
        self.transcript(first)
        self.transcript(second)
        with statements() as sql:
            body = self.client.get("/api/podcast/%s" % channel.id).get_json()
        self.assertEqual(len(body["episodes"]), 2)
        self.assertEqual(len(sql), 7, sql)

    def test_no_route_calls_a_provider(self):
        channel = self.channel()
        episode = self.episode(channel)
        self.transcript(episode)
        backlog = self.channel(title="Backlog")
        self.episode(backlog, duration=20)
        with mock.patch(
            "neezer.webui._dz_api", side_effect=AssertionError("provider called")
        ), mock.patch(
            "neezer.webui._dz_live", side_effect=AssertionError("provider called")
        ), mock.patch(
            "neezer.deezer.provider.DeezerProvider.dz",
            new_callable=mock.PropertyMock,
            side_effect=AssertionError("provider called"),
        ):
            self.assertEqual(
                self.client.get("/api/transcribe/status").status_code, 200
            )
            self.assertEqual(
                self.client.get("/api/transcribe/settings").status_code, 200
            )
            self.assertEqual(
                self.client.get(
                    "/api/podcast/episode/%s/transcript" % episode.id
                ).status_code,
                200,
            )
            self.assertEqual(
                self.client.get(
                    "/api/podcast/%s/transcript/search?q=memoire" % channel.id
                ).status_code,
                200,
            )
            self.assertEqual(
                self.client.get("/api/transcripts/search?q=memoire").status_code,
                200,
            )
            self.assertEqual(
                self.client.get("/api/podcast/%s" % channel.id).status_code, 200
            )
            response = self.client.post(
                "/api/podcast/%s/transcribe" % backlog.id,
                json={"auto": True, "backlog": "recent"},
            )
            self.assertEqual(response.status_code, 202, response.get_json())

    def test_a_remote_link_reads_and_never_changes(self):
        channel = self.channel()
        episode = self.episode(channel)
        self.transcript(episode)
        created = self.client.post(
            "/api/remote/links",
            json={"level": "read", "device": "test-device", "device_name": "Salon"},
        )
        self.assertEqual(created.status_code, 200, created.get_json())
        token = created.get_json()["link"]["token"]
        remote = self.app.test_client()
        self.assertEqual(
            remote.post("/api/remote/claim", json={"token": token}).status_code, 200
        )
        self.assertEqual(remote.get("/api/transcribe/status").status_code, 200)
        self.assertEqual(
            remote.get(
                "/api/podcast/episode/%s/transcript" % episode.id
            ).status_code,
            200,
        )
        self.assertEqual(
            remote.get(
                "/api/podcast/%s/transcript/search?q=memoire" % channel.id
            ).status_code,
            200,
        )
        self.assertEqual(
            remote.get("/api/transcripts/search?q=memoire").status_code, 200
        )
        for method, path, body in (
            ("GET", "/api/transcribe/settings", None),
            ("PUT", "/api/transcribe/settings", {"language": "en"}),
            ("POST", "/api/podcast/%s/transcribe" % channel.id, {"auto": True}),
            ("POST", "/api/podcast/episode/%s/transcribe" % episode.id, {}),
            ("POST", "/api/podcast/episode/%s/transcribe/stop" % episode.id, None),
            ("DELETE", "/api/podcast/episode/%s/transcript" % episode.id, None),
        ):
            response = remote.open(path, method=method, json=body)
            self.assertEqual(response.status_code, 403, (method, path))

    def test_the_blueprint_registers_its_remote_policy(self):
        self.assertIn("transcribe_api", self.app.blueprints)
        endpoints = {
            rule.endpoint.rsplit(".", 1)[-1]
            for rule in self.app.url_map.iter_rules()
            if rule.rule.startswith("/api/")
        }
        self.assertEqual(sorted(endpoints - set(POLICY)), [])
        self.assertEqual(sorted(set(POLICY) - endpoints), [])
        for name, level in (
            ("transcribe_status", "read"),
            ("episode_transcript", "read"),
            ("show_transcript_search", "read"),
            ("podcast_search", "read"),
            ("transcribe_settings", "never"),
            ("auto_transcribe", "never"),
            ("start_episode_transcript", "never"),
            ("stop_episode_transcript", "never"),
            ("delete_episode_transcript", "never"),
        ):
            self.assertEqual(POLICY[name], level, name)


if __name__ == "__main__":
    unittest.main()
