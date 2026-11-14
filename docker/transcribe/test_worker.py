#!/usr/bin/env python3
"""Contract tests for the sidecar worker, without Docker or a real engine.

Run: python3 docker/transcribe/test_worker.py -v
"""

from __future__ import annotations

import ast
import importlib.util
import json
import os
import re
import subprocess
import sys
import tempfile
import threading
import time
import unittest
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

WORKER = Path(__file__).with_name("worker.py")
ROOT = Path(__file__).resolve().parents[2]
COMPOSE = ROOT / "docker-compose.transcribe.yml"
DOCKERFILE = ROOT / "docker" / "transcribe" / "Dockerfile"

FAKE_WHISPER = r"""#!/usr/bin/env python3
import json
import os
import sys
import time

if os.environ.get("NEEZER_WORKER_TOKEN"):
    print("token leaked to the whisper child", file=sys.stderr)
    raise SystemExit(9)
args = sys.argv[1:]
if "--version" in args:
    print("whisper.cpp version: 1.9.5")
    raise SystemExit(0)
if "-dl" in args:
    print("whisper.cpp: auto-detected language: fr (p = 0.950000)", file=sys.stderr)
    raise SystemExit(0)
if "-of" not in args:
    raise SystemExit(2)
exit_code = int(os.environ.get("FAKE_WHISPER_EXIT", "0"))
if exit_code:
    print("boom", file=sys.stderr)
    raise SystemExit(exit_code)
prefix = args[args.index("-of") + 1]
sleep = float(os.environ.get("FAKE_WHISPER_SLEEP", "0"))
if sleep:
    time.sleep(sleep)
print("whisper_print_progress_callback: progress =  50%", file=sys.stderr)
payload = {
    "transcription": [
        {"offsets": {"from": 0, "to": 2000}, "text": " bonjour "},
        {"offsets": {"from": 3000, "to": 5000}, "text": "le monde"},
    ],
    "result": {"language": "fr"},
}
with open(prefix + ".json", "w", encoding="utf-8") as handle:
    json.dump(payload, handle)
"""

FAKE_FFMPEG = r"""#!/usr/bin/env python3
import os
import sys

if os.environ.get("NEEZER_WORKER_TOKEN"):
    print("token leaked to the ffmpeg child", file=sys.stderr)
    raise SystemExit(9)
args = sys.argv[1:]
if "-version" in args:
    print("ffmpeg version 9.0.2")
    raise SystemExit(0)
out = args[-1]
samples = b"\0" * 100
header = (
    b"RIFF" + (36 + len(samples)).to_bytes(4, "little") + b"WAVEfmt "
    + (16).to_bytes(4, "little") + (1).to_bytes(2, "little")
    + (1).to_bytes(2, "little") + (16000).to_bytes(4, "little")
    + (32000).to_bytes(4, "little") + (2).to_bytes(2, "little")
    + (16).to_bytes(2, "little") + b"data" + len(samples).to_bytes(4, "little")
)
with open(out, "wb") as handle:
    handle.write(header + samples)
"""


class State:
    def __init__(self, token: str, audio: bytes):
        self.token = token
        self.audio = audio
        self.claim: dict | None = None
        self.lease = ""
        self.duration_ms = 0
        self.done_ms = 0
        self.cancel_after: int | None = None
        self.pause_beats = 0
        self.chunk_500 = 0
        self.audio_partial_once = False
        self.audio_lease_expired = False
        self.chunk_attempts = 0
        self.chunk_payloads: list[dict] = []
        self.heartbeats = 0
        self.completes: list[dict] = []
        self.fails: list[dict] = []
        self.releases: list[dict] = []
        self.audio_ranges: list[str] = []
        self.chunk_beat = 0
        self.auth_ok = True
        self.lock = threading.Lock()


def handler_for(state: State):
    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.0"

        def log_message(self, *args):
            return

        def _json(self, status: int, payload: dict | None, headers=None):
            body = b"" if payload is None else json.dumps(payload).encode("utf-8")
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            for name, value in (headers or {}).items():
                self.send_header(name, value)
            self.end_headers()
            if body:
                self.wfile.write(body)

        def _auth(self):
            ok = self.headers.get("Authorization") == f"Bearer {state.token}"
            state.auth_ok = state.auth_ok and ok
            return ok

        def _body(self) -> dict:
            length = int(self.headers.get("Content-Length") or 0)
            raw = self.rfile.read(length) if length else b""
            if not raw:
                return {}
            try:
                value = json.loads(raw.decode("utf-8"))
            except ValueError:
                return {}
            return value if isinstance(value, dict) else {}

        def do_POST(self):
            if not self._auth():
                self._json(401, {"code": "unauthorized", "error": "jeton invalide"})
                return
            body = self._body()
            path = self.path.split("?", 1)[0]
            if path.endswith("/claim"):
                with state.lock:
                    claim = state.claim
                    state.claim = None
                if claim is None:
                    self._json(204, None)
                    return
                state.lease = claim["lease"]
                state.duration_ms = claim["duration_ms"]
                state.done_ms = claim["start_ms"]
                self._json(200, claim)
                return
            if path.endswith("/heartbeat"):
                with state.lock:
                    state.heartbeats += 1
                    beat = state.heartbeats
                    cancel = state.cancel_after is not None and beat >= state.cancel_after
                    pause = beat <= state.pause_beats
                self._json(
                    200,
                    {
                        "cancel": cancel,
                        "pause": pause,
                        "done_ms": state.done_ms,
                        "expires_at": "2026-11-15T00:00:00Z",
                    },
                )
                return
            if path.endswith("/chunk"):
                with state.lock:
                    state.chunk_attempts += 1
                    if state.chunk_500 > 0:
                        state.chunk_500 -= 1
                        self._json(500, {"code": "server", "error": "boom"})
                        return
                    state.chunk_payloads.append(body)
                    if not state.chunk_beat:
                        state.chunk_beat = state.heartbeats
                    state.done_ms = min(
                        int(body.get("start_ms") or 0)
                        + int(body.get("duration_ms") or 0),
                        state.duration_ms,
                    )
                    done = state.done_ms
                self._json(
                    200,
                    {
                        "applied": True,
                        "done_ms": done,
                        "n_segments": len(body.get("segments") or []),
                    },
                )
                return
            if path.endswith("/complete"):
                with state.lock:
                    state.completes.append(body)
                    done = state.done_ms
                    duration = state.duration_ms
                if done < duration - 5000:
                    self._json(
                        409,
                        {
                            "code": "incomplete",
                            "error": "incomplet",
                            "done_ms": done,
                            "duration_ms": duration,
                        },
                    )
                    return
                self._json(200, {"status": "ready"})
                return
            if path.endswith("/fail"):
                state.fails.append(body)
                self._json(200, {"status": "error"})
                return
            if path.endswith("/release"):
                state.releases.append(body)
                self._json(200, {"status": "stopped"})
                return
            self._json(404, {"code": "not_found", "error": "route inconnue"})

        def do_GET(self):
            if not self._auth():
                self._json(401, {"code": "unauthorized", "error": "jeton invalide"})
                return
            path = self.path.split("?", 1)[0]
            if "/audio/" not in path:
                self._json(404, {"code": "not_found", "error": "route inconnue"})
                return
            if self.headers.get("X-NS-Lease") != state.lease:
                self._json(403, {"code": "lease_expired", "error": "lease absent"})
                return
            if state.audio_lease_expired:
                self._json(403, {"code": "lease_expired", "error": "lease expiré"})
                return
            total = len(state.audio)
            header = self.headers.get("Range") or ""
            match = re.match(r"bytes=(\d+)-$", header)
            state.audio_ranges.append(header)
            if not match:
                self._json(416, {"code": "range", "error": "range requis"})
                return
            offset = int(match.group(1))
            if offset >= total:
                self._json(
                    416,
                    {"code": "range", "error": "hors bornes"},
                    {"Content-Range": f"bytes */{total}"},
                )
                return
            if state.audio_partial_once and offset == 0:
                state.audio_partial_once = False
                end = max(0, total // 8 - 1)
                body = state.audio[: end + 1]
                self.send_response(206)
                self.send_header("Content-Type", "audio/flac")
                self.send_header("Content-Range", f"bytes 0-{end}/{total}")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
                return
            body = state.audio[offset:]
            self.send_response(206)
            self.send_header("Content-Type", "audio/flac")
            self.send_header("Content-Range", f"bytes {offset}-{total - 1}/{total}")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

    return Handler


class WorkerCase(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="neezer-worker-")
        self.addCleanup(self.temp.cleanup)
        self.dir = Path(self.temp.name)
        self.model = self.dir / "model.bin"
        self.model.write_bytes(b"ggml" + b"\0" * (1 << 20))
        self.whisper = self.dir / "fake-whisper"
        self.whisper.write_text(FAKE_WHISPER, encoding="utf-8")
        self.whisper.chmod(0o755)
        self.ffmpeg = self.dir / "fake-ffmpeg"
        self.ffmpeg.write_text(FAKE_FFMPEG, encoding="utf-8")
        self.ffmpeg.chmod(0o755)
        self.work = self.dir / "work"
        token = "test-token-" + uuid.uuid4().hex
        audio = (b"neezer-audio-" * 65536)[: 1 << 20]
        self.state = State(token, audio)
        self.server = ThreadingHTTPServer(
            ("127.0.0.1", 0), handler_for(self.state)
        )
        self.server.daemon_threads = True
        self.thread = threading.Thread(
            target=self.server.serve_forever, name="fake-neezer", daemon=True
        )
        self.thread.start()

        def stop_server():
            self.server.shutdown()
            self.server.server_close()
            self.thread.join(timeout=2)

        self.addCleanup(stop_server)
        self.base = f"http://127.0.0.1:{self.server.server_port}"

    def make_claim(self, duration_ms=1_200_000, start_ms=0, language="fr"):
        episode_id = str(uuid.uuid4())
        lease = "lease-" + uuid.uuid4().hex
        self.state.claim = {
            "episode_id": episode_id,
            "lease": lease,
            "audio_url": f"/api/transcribe/worker/audio/{episode_id}",
            "duration_ms": duration_ms,
            "language": language,
            "chunk_ms": 600_000,
            "overlap_ms": 2_000,
            "max_minutes": 360,
            "start_ms": start_ms,
            "expires_at": "2026-11-15T00:00:00Z",
        }
        return self.state.claim

    def run_worker(self, args=("--once",), extra=None):
        env = {
            **os.environ,
            "NEEZER_URL": self.base,
            "NEEZER_WORKER_TOKEN": self.state.token,
            "MODEL": str(self.model),
            "WHISPER_BIN": str(self.whisper),
            "FFMPEG": str(self.ffmpeg),
            "NEEZER_TMP_DIR": str(self.work),
            "NEEZER_HEARTBEAT_SECONDS": "0.05",
            "NEEZER_RETRY_BASE": "0.02",
            "NEEZER_RETRY_MAX": "0.05",
            "NEEZER_RETRY_ATTEMPTS": "4",
            "THREADS": "1",
            "CHUNK_SECONDS": "600",
            "LANGUAGE": "auto",
            "LOG_LEVEL": "DEBUG",
        }
        if extra:
            env.update(extra)
        return subprocess.run(
            [sys.executable, str(WORKER), *args],
            env=env,
            capture_output=True,
            text=True,
            timeout=30,
        )

    def test_happy_path_chunks_and_overlap(self):
        self.make_claim()
        result = self.run_worker()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(self.state.auth_ok)
        self.assertEqual(len(self.state.chunk_payloads), 2)
        first, second = self.state.chunk_payloads
        self.assertEqual(first["start_ms"], 0)
        self.assertEqual(first["duration_ms"], 600_000)
        self.assertEqual(len(first["segments"]), 2)
        self.assertEqual(first["engine"], "whisper.cpp")
        self.assertEqual(first["engine_version"], "1.9.5")
        self.assertEqual(second["start_ms"], 600_000)
        self.assertEqual(len(second["segments"]), 1)
        self.assertEqual(second["segments"][0]["start_ms"], 603_000)
        self.assertEqual(second["segments"][0]["end_ms"], 605_000)
        self.assertEqual(len(self.state.completes), 1)
        self.assertEqual(self.state.fails, [])
        self.assertEqual(self.state.audio_ranges[0], "bytes=0-")
        self.assertFalse(self.work.exists() and any(self.work.iterdir()))
        self.assertNotIn(self.state.token, result.stdout + result.stderr)

    def test_resume_from_done_ms(self):
        self.make_claim(duration_ms=1_200_000, start_ms=600_000)
        result = self.run_worker()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(self.state.chunk_payloads), 1)
        self.assertEqual(self.state.chunk_payloads[0]["start_ms"], 600_000)
        self.assertEqual(len(self.state.completes), 1)

    def test_cancel_kills_the_engine_and_releases(self):
        self.make_claim(duration_ms=600_000)
        self.state.cancel_after = 8
        started = time.monotonic()
        result = self.run_worker(extra={"FAKE_WHISPER_SLEEP": "8"})
        elapsed = time.monotonic() - started
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertLess(elapsed, 5.0)
        self.assertEqual(len(self.state.releases), 1)
        self.assertEqual(self.state.chunk_payloads, [])
        self.assertEqual(self.state.fails, [])
        self.assertFalse(self.work.exists() and any(self.work.iterdir()))

    def test_chunk_retries_on_5xx(self):
        self.make_claim(duration_ms=600_000)
        self.state.chunk_500 = 2
        result = self.run_worker()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.state.chunk_attempts, 3)
        self.assertEqual(len(self.state.chunk_payloads), 1)
        self.assertEqual(len(self.state.completes), 1)

    def test_audio_lease_expiry_drops_the_job_without_failing_it(self):
        self.make_claim(duration_ms=600_000)
        self.state.audio_lease_expired = True
        result = self.run_worker()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.state.chunk_payloads, [])
        self.assertEqual(self.state.fails, [])
        self.assertEqual(self.state.completes, [])

    def test_bad_token_exits_nonzero_without_logging_it(self):
        self.make_claim(duration_ms=600_000)
        wrong = "wrong-token-" + uuid.uuid4().hex
        result = self.run_worker(extra={"NEEZER_WORKER_TOKEN": wrong})
        self.assertEqual(result.returncode, 2)
        self.assertEqual(self.state.chunk_payloads, [])
        self.assertNotIn(wrong, result.stdout + result.stderr)

    def test_engine_failure_reports_fail(self):
        self.make_claim(duration_ms=600_000)
        result = self.run_worker(extra={"FAKE_WHISPER_EXIT": "3"})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.state.chunk_payloads, [])
        self.assertEqual(len(self.state.fails), 1)
        self.assertEqual(self.state.fails[0]["code"], "exit")
        self.assertIn("boom", self.state.fails[0]["message"])
        self.assertEqual(self.state.completes, [])

    def test_audio_range_resumes_after_a_partial_response(self):
        self.make_claim(duration_ms=600_000)
        self.state.audio_partial_once = True
        result = self.run_worker()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertGreaterEqual(len(self.state.audio_ranges), 2)
        self.assertEqual(self.state.audio_ranges[0], "bytes=0-")
        self.assertEqual(
            self.state.audio_ranges[1], f"bytes={len(self.state.audio) // 8}-"
        )
        self.assertEqual(len(self.state.completes), 1)
        self.assertEqual(self.state.fails, [])

    def test_language_detection_pins_the_chunk_language(self):
        self.make_claim(duration_ms=600_000, language="auto")
        result = self.run_worker()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(self.state.chunk_payloads), 1)
        self.assertEqual(self.state.chunk_payloads[0]["language"], "fr")

    def test_pause_delays_the_chunk_without_failing(self):
        self.make_claim(duration_ms=600_000)
        self.state.pause_beats = 20
        result = self.run_worker()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertGreaterEqual(self.state.heartbeats, 20)
        self.assertGreaterEqual(self.state.chunk_beat, 20)
        self.assertEqual(len(self.state.chunk_payloads), 1)
        self.assertEqual(len(self.state.completes), 1)

    def test_check_names_a_missing_model_and_never_calls_the_api(self):
        result = self.run_worker(
            args=("--check",), extra={"MODEL": str(self.dir / "absent.bin")}
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("model_missing", result.stderr)
        self.assertIsNone(self.state.claim)
        self.assertEqual(self.state.heartbeats, 0)
        self.assertEqual(self.state.audio_ranges, [])

    def test_check_accepts_a_valid_model(self):
        result = self.run_worker(args=("--check",))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("ok model", result.stdout)
        self.assertIn("ok engine", result.stdout)
        self.assertIn("ok ffmpeg", result.stdout)
        self.assertNotIn(self.state.token, result.stdout + result.stderr)

    def test_worker_imports_no_database_and_no_archive(self):
        tree = ast.parse(WORKER.read_text(encoding="utf-8"))
        modules = set()
        for node in ast.walk(tree):
            if isinstance(node, ast.Import):
                modules.update(alias.name.split(".")[0] for alias in node.names)
            elif isinstance(node, ast.ImportFrom) and node.module:
                modules.add(node.module.split(".")[0])
        allowed = {
            "__future__",
            "argparse",
            "ctypes",
            "dataclasses",
            "json",
            "logging",
            "os",
            "re",
            "requests",
            "shutil",
            "signal",
            "subprocess",
            "sys",
            "tempfile",
            "threading",
            "time",
            "typing",
            "urllib",
            "uuid",
        }
        self.assertLessEqual(modules, allowed)
        source = WORKER.read_text(encoding="utf-8")
        for forbidden in ("sqlite3", "peewee", "psycopg", "pymysql"):
            self.assertNotIn(forbidden, modules)
        self.assertIn("-ng", source)
        self.assertIn("X-NS-Lease", source)
        self.assertIn("Authorization", source)

    @unittest.skipUnless(
        importlib.util.find_spec("yaml"), "PyYAML absent"
    )
    def test_compose_contract(self):
        import yaml

        data = yaml.safe_load(COMPOSE.read_text(encoding="utf-8"))
        service = data["services"]["transcriber"]
        self.assertTrue(service["read_only"])
        self.assertEqual(service["cap_drop"], ["ALL"])
        self.assertIn("no-new-privileges:true", service["security_opt"])
        self.assertEqual(service["user"], "10001:10001")
        self.assertIn("mem_limit", service)
        self.assertEqual(service["mem_limit"], service["memswap_limit"])
        self.assertIn("cpus", service)
        self.assertIn("./models:/models:ro", service["volumes"])
        self.assertEqual(service["networks"], ["internal"])
        self.assertNotIn("ports", service)
        self.assertIn("NEEZER_WORKER_TOKEN_FILE", service["environment"])
        self.assertNotIn("NEEZER_WORKER_TOKEN", service["environment"])
        self.assertTrue(data["networks"]["internal"]["internal"])
        self.assertIn("transcriber_token", data["secrets"])
        self.assertIn("neezer", data["services"])

    def test_dockerfile_contract(self):
        text = DOCKERFILE.read_text(encoding="utf-8")
        for needle in (
            "WHISPER_VERSION=1.9.5",
            "GGML_NATIVE=OFF",
            "BUILD_SHARED_LIBS=OFF",
            "WHISPER_BUILD_IS_DEV=OFF",
            "mwader/static-ffmpeg",
            "USER 10001:10001",
            "/out/whisper-cli",
            "python3",
        ):
            self.assertIn(needle, text)
        self.assertNotIn("fetch-whisper-model", text)
        self.assertNotIn("ADD http", text)

    def test_fetch_script_pins_both_checksums(self):
        script = ROOT / "tools" / "fetch-whisper-model.sh"
        self.assertTrue(os.access(script, os.X_OK))
        text = script.read_text(encoding="utf-8")
        for needle in (
            "317eb69c11673c9de1e1f0d459b253999804ec71ac4c23c17ecf5fbe24e259a1",
            "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
            "874188075",
            "487601967",
            "huggingface.co/ggerganov/whisper.cpp",
            "/.dockerenv",
        ):
            self.assertIn(needle, text)


if __name__ == "__main__":
    unittest.main(verbosity=2)
