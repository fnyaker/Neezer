#!/usr/bin/env python3
"""Neezer transcription sidecar: lease a job over HTTP, transcribe it, report text.

The worker knows exactly four things: an HTTP base URL, a device token, a
GGML model file and the two binaries it runs. It never opens the database,
never mounts the archive, never calls a provider and never downloads a model.
Audio is fetched from the leased episode's ``audio_url`` with ``Range`` and
written to a temporary file; only the leased episode is ever readable.
"""

from __future__ import annotations

import argparse
import ctypes
import json
import logging
import os
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
from dataclasses import dataclass
from typing import Any, Callable, Optional
from urllib.parse import urlsplit

import requests

ENGINE = "whisper.cpp"
PROTOCOL = 1
CHUNK_SECONDS = 600
OVERLAP_SECONDS = 2
SAMPLE_RATE = 16000
MAX_SEGMENTS = 2000
MAX_TEXT = 2000
MAX_AUDIO_BYTES = 1 << 30
MIN_MODEL_BYTES = 1 << 20
MODEL_MAGICS = (b"ggml", b"lmgg")
DETECT_SECONDS = 120
DETECT_MIN_PROBABILITY = 0.6
TEMP_MAX_AGE = 24 * 3600
DEFAULT_MODEL = "/models/ggml-large-v3-turbo-q8_0.bin"

_PROGRESS = re.compile(r"progress\s*=\s*(\d{1,3})\s*%")
_DETECTED = re.compile(
    r"auto-detected language:\s*([a-z]{2,3})\s*\(\s*p\s*=\s*([0-9.]+)\s*\)"
)
_CONTENT_RANGE = re.compile(r"bytes\s+(\d+)-(\d+)/(\d+|\*)")
_SEMVER = re.compile(r"(\d+\.\d+\.\d+)")
_CONTROL = re.compile(r"[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]")
_SPACES = re.compile(r"\s+")
_TAG_ONLY = re.compile(r"^\[[^\]]*\]$")
_LANGUAGE = re.compile(r"^[a-z]{2,3}$")
_UUID = re.compile(
    r"^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[1-8][0-9a-fA-F]{3}-"
    r"[89abAB][0-9a-fA-F]{3}-[0-9a-fA-F]{12}$"
)

LOG = logging.getLogger("neezer.transcribe.worker")


class ConfigError(RuntimeError):
    pass


class EngineFailure(RuntimeError):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code
        self.message = message


class Cancelled(RuntimeError):
    pass


class AuthError(RuntimeError):
    def __init__(self, status: int, code: str, message: str):
        super().__init__(message)
        self.status = status
        self.code = code
        self.message = message


class HttpError(RuntimeError):
    def __init__(self, status: int, payload: Optional[dict] = None):
        payload = payload or {}
        self.status = status
        self.payload = payload
        self.code = str(payload.get("code") or "")
        self.message = str(payload.get("error") or f"HTTP {status}")
        super().__init__(self.message)


@dataclass(frozen=True)
class Config:
    url: str
    token: str
    token_source: str
    model: str
    language: str
    threads: int
    chunk_seconds: int
    whisper: str
    ffmpeg: str
    tmp_root: str
    poll_seconds: float = 5.0
    retry_base: float = 1.0
    retry_max: float = 30.0
    retry_attempts: int = 5
    heartbeat_seconds: float = 5.0
    audio_timeout: float = 120.0
    log_level: str = "INFO"

    @classmethod
    def from_env(cls, env: Optional[dict] = None) -> "Config":
        values = os.environ if env is None else env
        raw_url = str(values.get("NEEZER_URL") or "").strip()
        if not raw_url:
            raise ConfigError("NEEZER_URL est requis (ex. http://neezer:5722)")
        raw_url = raw_url.rstrip("/")
        parts = urlsplit(raw_url)
        if parts.scheme not in ("http", "https") or not parts.netloc:
            raise ConfigError("NEEZER_URL doit être une URL http(s) absolue")
        if parts.username or parts.password:
            raise ConfigError("NEEZER_URL ne doit pas contenir d'identifiants")
        if parts.query or parts.fragment:
            raise ConfigError("NEEZER_URL ne doit pas contenir de requête")

        token_file = str(values.get("NEEZER_WORKER_TOKEN_FILE") or "").strip()
        if token_file:
            try:
                with open(token_file, encoding="utf-8") as handle:
                    token = handle.read().strip()
            except OSError as exc:
                raise ConfigError(
                    f"jeton illisible dans NEEZER_WORKER_TOKEN_FILE ({exc.strerror})"
                ) from exc
            token_source = f"file:{token_file}"
        else:
            token = str(values.get("NEEZER_WORKER_TOKEN") or "").strip()
            token_source = "environment"
        if not token:
            raise ConfigError(
                "NEEZER_WORKER_TOKEN ou NEEZER_WORKER_TOKEN_FILE est requis"
            )
        if len(token) > 200 or any(char.isspace() for char in token):
            raise ConfigError("le jeton d'appareil est invalide")

        language = str(values.get("LANGUAGE") or "auto").strip().lower()
        if language != "auto" and not _LANGUAGE.match(language):
            language = "auto"

        return cls(
            url=raw_url,
            token=token,
            token_source=token_source,
            model=str(values.get("MODEL") or DEFAULT_MODEL).strip(),
            language=language,
            threads=_env_int(values, "THREADS", 2, 1, 16),
            chunk_seconds=_env_int(values, "CHUNK_SECONDS", CHUNK_SECONDS, 60, 3600),
            whisper=str(values.get("WHISPER_BIN") or "whisper-cli").strip(),
            ffmpeg=str(values.get("FFMPEG") or "ffmpeg").strip(),
            tmp_root=str(
                values.get("NEEZER_TMP_DIR")
                or os.path.join(tempfile.gettempdir(), "neezer-transcribe")
            ).strip(),
            poll_seconds=_env_float(values, "POLL_SECONDS", 5.0, 0.05, 300.0),
            retry_base=_env_float(values, "NEEZER_RETRY_BASE", 1.0, 0.0, 60.0),
            retry_max=_env_float(values, "NEEZER_RETRY_MAX", 30.0, 0.0, 300.0),
            retry_attempts=_env_int(values, "NEEZER_RETRY_ATTEMPTS", 5, 1, 20),
            heartbeat_seconds=_env_float(
                values, "NEEZER_HEARTBEAT_SECONDS", 5.0, 0.05, 60.0
            ),
            audio_timeout=_env_float(values, "NEEZER_AUDIO_TIMEOUT", 120.0, 5.0, 3600.0),
            log_level=str(values.get("LOG_LEVEL") or "INFO").strip().upper(),
        )


def _env_int(env: dict, name: str, default: int, low: int, high: int) -> int:
    try:
        value = int(str(env.get(name, "")).strip())
    except (TypeError, ValueError):
        return default
    return max(low, min(high, value))


def _env_float(env: dict, name: str, default: float, low: float, high: float) -> float:
    try:
        value = float(str(env.get(name, "")).strip())
    except (TypeError, ValueError):
        return default
    if value != value:
        return default
    return max(low, min(high, value))


def resolve_binary(name: str) -> Optional[str]:
    if os.path.sep in name:
        return name if os.path.isfile(name) and os.access(name, os.X_OK) else None
    return shutil.which(name)


def child_env() -> dict:
    env = dict(os.environ)
    env.pop("NEEZER_WORKER_TOKEN", None)
    return env


def model_problem(path: str) -> Optional[tuple[str, str]]:
    if not os.path.isfile(path):
        return (
            "model_missing",
            f"{path} est absent ; exécutez tools/fetch-whisper-model.sh "
            "sur l'hôte puis montez le dossier en lecture seule sur /models",
        )
    try:
        size = os.path.getsize(path)
        with open(path, "rb") as handle:
            magic = handle.read(4)
    except OSError:
        return ("model_invalid", f"{path} est illisible")
    if size < MIN_MODEL_BYTES:
        return ("model_invalid", f"{path} est trop petit ({size} octets)")
    if magic not in MODEL_MAGICS:
        return ("model_invalid", f"{path} n'a pas la magie GGML (téléchargement incomplet ?)")
    return None


def probe_engine(path: str) -> Optional[tuple[str, str]]:
    resolved = resolve_binary(path)
    if resolved is None:
        return ("engine_missing", f"whisper-cli introuvable ou non exécutable ({path})")
    try:
        proc = subprocess.run(
            [resolved, "--version"],
            stdin=subprocess.DEVNULL,
            capture_output=True,
            timeout=10,
            check=False,
            env=child_env(),
        )
    except (OSError, subprocess.SubprocessError):
        return ("engine_error", "whisper-cli ne répond pas")
    if proc.returncode != 0:
        return ("engine_error", "whisper-cli --version a échoué")
    text = (proc.stdout or b"").decode("utf-8", "replace").strip()
    match = _SEMVER.search(text)
    return (None, match.group(1) if match else text.splitlines()[0][:32] if text else "")


def probe_ffmpeg(path: str) -> Optional[tuple[str, str]]:
    resolved = resolve_binary(path)
    if resolved is None:
        return ("ffmpeg_missing", f"ffmpeg introuvable ou non exécutable ({path})")
    try:
        proc = subprocess.run(
            [resolved, "-version"],
            stdin=subprocess.DEVNULL,
            capture_output=True,
            timeout=10,
            check=False,
            env=child_env(),
        )
    except (OSError, subprocess.SubprocessError):
        return ("ffmpeg_error", "ffmpeg ne répond pas")
    if proc.returncode != 0:
        return ("ffmpeg_error", "ffmpeg -version a échoué")
    return (None, "")


def preflight(config: Config) -> tuple[list[tuple[str, str]], str]:
    errors: list[tuple[str, str]] = []
    problem = model_problem(config.model)
    if problem:
        errors.append(problem)
    engine_problem, version = probe_engine(config.whisper)
    if engine_problem:
        errors.append(engine_problem)
    ffmpeg_problem, _ = probe_ffmpeg(config.ffmpeg)
    if ffmpeg_problem:
        errors.append(ffmpeg_problem)
    return errors, version


class Client:
    """The worker API, with bounded exponential backoff and no token in any log."""

    def __init__(self, config: Config):
        self.base = config.url
        self.token = config.token
        self.retry_base = config.retry_base
        self.retry_max = config.retry_max
        self.retry_attempts = config.retry_attempts
        self.audio_timeout = config.audio_timeout
        self.session = requests.Session()
        self.session.headers.update(
            {
                "User-Agent": f"neezer-transcriber/{PROTOCOL}",
                "Accept": "application/json",
            }
        )

    def close(self) -> None:
        self.session.close()

    def url(self, path: str) -> str:
        if not isinstance(path, str) or not path.startswith("/"):
            raise HttpError(0, {"code": "bad_url", "error": "URL d'audio invalide"})
        return self.base + path

    def _headers(self, lease: Optional[str] = None) -> dict:
        headers = {"Authorization": f"Bearer {self.token}"}
        if lease:
            headers["X-NS-Lease"] = lease
        return headers

    def _backoff(self, attempt: int, retry_after: Optional[str]) -> None:
        delay = min(self.retry_max, self.retry_base * (2 ** max(0, attempt - 1)))
        if retry_after:
            try:
                delay = max(delay, min(120.0, float(retry_after)))
            except (TypeError, ValueError):
                pass
        if delay > 0:
            time.sleep(delay)

    def request(
        self,
        method: str,
        path: str,
        body: Optional[dict] = None,
        lease: Optional[str] = None,
        expected: tuple[int, ...] = (200,),
    ) -> Optional[dict]:
        attempt = 0
        while True:
            attempt += 1
            try:
                response = self.session.request(
                    method,
                    self.url(path),
                    json=body,
                    headers=self._headers(lease),
                    timeout=(10, 120),
                    allow_redirects=False,
                )
            except requests.RequestException as exc:
                if attempt >= self.retry_attempts:
                    raise HttpError(
                        0, {"code": "network", "error": type(exc).__name__}
                    ) from exc
                LOG.warning(
                    "%s %s: réseau (%s), nouvelle tentative %d/%d",
                    method,
                    path,
                    type(exc).__name__,
                    attempt,
                    self.retry_attempts,
                )
                self._backoff(attempt, None)
                continue
            if response.status_code in expected:
                if response.status_code == 204 or not response.content:
                    return None
                try:
                    payload = response.json()
                except ValueError:
                    return None
                return payload if isinstance(payload, dict) else None
            payload = None
            try:
                candidate = response.json()
                payload = candidate if isinstance(candidate, dict) else None
            except ValueError:
                payload = None
            if response.status_code in (401, 403):
                response.close()
                raise AuthError(
                    response.status_code,
                    str((payload or {}).get("code") or "unauthorized"),
                    str((payload or {}).get("error") or "jeton refusé"),
                )
            if response.status_code == 429 or 500 <= response.status_code < 600:
                if attempt >= self.retry_attempts:
                    response.close()
                    raise HttpError(response.status_code, payload)
                LOG.warning(
                    "%s %s: HTTP %d, nouvelle tentative %d/%d",
                    method,
                    path,
                    response.status_code,
                    attempt,
                    self.retry_attempts,
                )
                retry_after = response.headers.get("Retry-After")
                response.close()
                self._backoff(attempt, retry_after)
                continue
            response.close()
            raise HttpError(response.status_code, payload)

    def claim(self) -> Optional[dict]:
        payload = self.request(
            "POST", "/api/transcribe/worker/claim", expected=(200, 204)
        )
        if payload is None:
            return None
        if not payload.get("lease") or not payload.get("episode_id"):
            raise HttpError(200, {"code": "bad_claim", "error": "claim incomplet"})
        return payload

    def heartbeat(self, episode_id: str, lease: str, chunk_pct: Optional[float]) -> dict:
        body = {} if chunk_pct is None else {"chunk_pct": round(float(chunk_pct), 1)}
        return (
            self.request(
                "POST",
                f"/api/transcribe/worker/{episode_id}/heartbeat",
                body,
                lease=lease,
            )
            or {}
        )

    def chunk(self, episode_id: str, lease: str, body: dict) -> dict:
        return (
            self.request(
                "POST",
                f"/api/transcribe/worker/{episode_id}/chunk",
                body,
                lease=lease,
            )
            or {}
        )

    def complete(self, episode_id: str, lease: str, body: dict) -> dict:
        return (
            self.request(
                "POST",
                f"/api/transcribe/worker/{episode_id}/complete",
                body,
                lease=lease,
            )
            or {}
        )

    def fail(self, episode_id: str, lease: str, code: str, message: str) -> dict:
        return (
            self.request(
                "POST",
                f"/api/transcribe/worker/{episode_id}/fail",
                {"code": code[:24], "message": message[:240]},
                lease=lease,
            )
            or {}
        )

    def release(self, episode_id: str, lease: str) -> dict:
        return (
            self.request(
                "POST",
                f"/api/transcribe/worker/{episode_id}/release",
                {},
                lease=lease,
            )
            or {}
        )


class JobState:
    def __init__(self, stop: threading.Event):
        self.stop = stop
        self.cancel = threading.Event()
        self.pause = threading.Event()
        self.lost = threading.Event()
        self.revoked = threading.Event()
        self._lock = threading.Lock()
        self._pct = 0.0

    def stopping(self) -> bool:
        return bool(
            self.stop.is_set()
            or self.cancel.is_set()
            or self.lost.is_set()
            or self.revoked.is_set()
        )

    def progress(self, value: float) -> None:
        with self._lock:
            self._pct = max(0.0, min(100.0, float(value)))

    def pct(self) -> float:
        with self._lock:
            return self._pct


class Heartbeat(threading.Thread):
    """Keeps the lease alive and carries the server's pause/cancel decisions."""

    def __init__(self, client: Client, episode_id: str, lease: str, state: JobState,
                 interval: float):
        super().__init__(name="transcribe-heartbeat", daemon=True)
        self.client = client
        self.episode_id = episode_id
        self.lease = lease
        self.state = state
        self.interval = interval
        self._stop = threading.Event()

    def stop(self) -> None:
        self._stop.set()

    def run(self) -> None:
        failures = 0
        try:
            while not self._stop.wait(self.interval):
                try:
                    beat = self.client.heartbeat(
                        self.episode_id, self.lease, self.state.pct()
                    )
                except AuthError:
                    self.state.revoked.set()
                    return
                except HttpError as exc:
                    if exc.status == 409 or exc.code in (
                        "lease_expired",
                        "lease_required",
                    ):
                        self.state.lost.set()
                        return
                    failures += 1
                    if failures == 1 or failures % 12 == 0:
                        LOG.warning("heartbeat: %s", exc)
                    continue
                failures = 0
                if beat.get("cancel"):
                    self.state.cancel.set()
                if beat.get("pause"):
                    self.state.pause.set()
                else:
                    self.state.pause.clear()
        finally:
            self.client.close()


def _preexec():
    if not sys.platform.startswith("linux"):
        return None

    def hook():
        try:
            libc = ctypes.CDLL(None, use_errno=True)
            libc.prctl(1, signal.SIGKILL, 0, 0, 0)
        except Exception:
            pass

    return hook


def _kill_group(proc: subprocess.Popen, number: int) -> None:
    try:
        os.killpg(os.getpgid(proc.pid), number)
    except (OSError, AttributeError):
        pass


def _terminate(proc: subprocess.Popen) -> None:
    _kill_group(proc, signal.SIGCONT)
    _kill_group(proc, signal.SIGTERM)
    try:
        proc.wait(timeout=5)
        return
    except subprocess.TimeoutExpired:
        pass
    _kill_group(proc, signal.SIGKILL)
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        pass


def run_process(
    cmd: list[str],
    timeout: float,
    state: JobState,
    on_stderr: Optional[Callable[[str], None]] = None,
) -> None:
    proc = subprocess.Popen(
        cmd,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        start_new_session=True,
        preexec_fn=_preexec(),
        env=child_env(),
    )
    tail: list[str] = []

    def read_stderr() -> None:
        try:
            for raw in proc.stderr:
                line = raw.decode("utf-8", "replace").rstrip()
                tail.append(line)
                if len(tail) > 20:
                    tail.pop(0)
                if on_stderr is not None:
                    try:
                        on_stderr(line)
                    except Exception:
                        LOG.debug("hook stderr en échec", exc_info=True)
        except (OSError, ValueError):
            pass

    reader = threading.Thread(target=read_stderr, daemon=True)
    reader.start()
    started = time.monotonic()
    paused_for = 0.0
    paused_at: Optional[float] = None
    paused = False
    try:
        while True:
            code = proc.poll()
            if code is not None:
                break
            if state.stopping():
                _terminate(proc)
                raise Cancelled()
            if state.pause.is_set():
                if not paused:
                    _kill_group(proc, signal.SIGSTOP)
                    paused = True
                    paused_at = time.monotonic()
            elif paused:
                paused_for += time.monotonic() - (paused_at or time.monotonic())
                paused = False
                paused_at = None
                _kill_group(proc, signal.SIGCONT)
            elapsed = time.monotonic() - started - paused_for
            if elapsed > timeout:
                _terminate(proc)
                raise EngineFailure("timeout", "le moteur a dépassé son délai")
            time.sleep(0.2)
    finally:
        if paused:
            _kill_group(proc, signal.SIGCONT)
        reader.join(1.0)
        try:
            proc.stderr.close()
        except (AttributeError, OSError):
            pass
    if code != 0:
        detail = tail[-1] if tail else f"code {code}"
        raise EngineFailure("exit", f"le moteur s'est arrêté ({detail})"[:240])


def extract_chunk(config: Config, audio: str, start_ms: int, length_ms: int,
                  out_wav: str, state: JobState) -> str:
    seconds = max(0.001, length_ms / 1000.0)
    cmd = [
        config.ffmpeg,
        "-nostdin",
        "-v",
        "error",
        "-ss",
        f"{max(0, start_ms) / 1000.0:.3f}",
        "-t",
        f"{seconds:.3f}",
        "-i",
        audio,
        "-map",
        "0:a:0",
        "-ac",
        "1",
        "-ar",
        str(SAMPLE_RATE),
        "-c:a",
        "pcm_s16le",
        "-f",
        "wav",
        "-y",
        out_wav,
    ]
    try:
        run_process(cmd, timeout=max(120.0, seconds * 2.0), state=state)
    except Cancelled:
        raise
    except EngineFailure as exc:
        if exc.code == "timeout":
            raise
        raise EngineFailure("no_audio", f"ffmpeg: {exc.message}"[:240]) from exc
    if not os.path.isfile(out_wav) or os.path.getsize(out_wav) <= 44:
        raise EngineFailure("no_audio", "l'extrait audio est vide")
    return out_wav


def clean_text(raw: Any) -> Optional[str]:
    if not isinstance(raw, str):
        return None
    text = _SPACES.sub(" ", _CONTROL.sub(" ", raw)).strip()
    if not text or _TAG_ONLY.match(text):
        return None
    return text[:MAX_TEXT].strip() or None


def _milliseconds(value: Any) -> Optional[int]:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return None
    return int(round(value))


def normalize_language(value: Any) -> Optional[str]:
    text = str(value or "").strip().lower()
    return text if text == "auto" or _LANGUAGE.match(text) else None


def fit_segments(segments: list[dict], budget: int = 800_000) -> list[dict]:
    kept: list[dict] = []
    used = 0
    for segment in segments:
        size = len(segment["text"].encode("utf-8")) + 64
        if used + size > budget:
            break
        kept.append(segment)
        used += size
    return kept


def normalize_segments(items: Any, chunk_start_ms: int, chunk_length_ms: int,
                       overlap_ms: int) -> list[dict]:
    start = max(0, int(chunk_start_ms))
    end = start + max(1, int(chunk_length_ms))
    cutoff = start + max(0, int(overlap_ms)) if start > 0 else 0
    out: list[dict] = []
    for item in items if isinstance(items, list) else ():
        if len(out) >= MAX_SEGMENTS:
            break
        if not isinstance(item, dict):
            continue
        offsets = item.get("offsets")
        if not isinstance(offsets, dict):
            continue
        segment_start = _milliseconds(offsets.get("from"))
        segment_end = _milliseconds(offsets.get("to"))
        if segment_start is None or segment_end is None:
            continue
        segment_start = max(0, segment_start) + start
        segment_end = max(0, segment_end) + start
        if segment_end <= segment_start:
            continue
        if cutoff and segment_end <= cutoff:
            continue
        segment_start = max(segment_start, start)
        segment_end = min(segment_end, end)
        if segment_end <= segment_start:
            continue
        text = clean_text(item.get("text"))
        if not text:
            continue
        out.append(
            {"start_ms": segment_start, "end_ms": segment_end, "text": text}
        )
    out.sort(key=lambda segment: (segment["start_ms"], segment["end_ms"]))
    return out


def recognize(config: Config, wav: str, prefix: str, language: str,
              length_ms: int, state: JobState) -> tuple[list, Optional[str]]:
    cmd = [
        config.whisper,
        "-m",
        config.model,
        "-f",
        wav,
        "-oj",
        "-of",
        prefix,
        "-l",
        normalize_language(language) or "auto",
        "-t",
        str(config.threads),
        "-ng",
        "-pp",
        "-sns",
    ]

    def stderr(line: str) -> None:
        match = _PROGRESS.search(line)
        if match:
            state.progress(int(match.group(1)))

    run_process(
        cmd,
        timeout=max(900.0, (length_ms / 1000.0) * 6.0),
        state=state,
        on_stderr=stderr,
    )
    path = prefix + ".json"
    try:
        with open(path, encoding="utf-8") as handle:
            data = json.load(handle)
    except (OSError, ValueError) as exc:
        raise EngineFailure("bad_json", "le JSON du moteur est illisible") from exc
    if not isinstance(data, dict) or not isinstance(data.get("transcription"), list):
        raise EngineFailure("bad_json", "le JSON du moteur n'a pas de transcription")
    result = data.get("result")
    detected = None
    if isinstance(result, dict):
        detected = normalize_language(result.get("language"))
    return data["transcription"], detected


def detect_language(config: Config, audio: str, workdir: str, duration_ms: int,
                    state: JobState) -> Optional[str]:
    if duration_ms < 5000:
        return None
    starts = [0]
    if duration_ms > DETECT_SECONDS * 1000 * 2:
        starts.append(int(duration_ms * 0.25))
    for index, start in enumerate(starts):
        wav = os.path.join(workdir, f"detect-{index}.wav")
        length = min(DETECT_SECONDS * 1000, duration_ms - start)
        found: dict[str, Any] = {"language": None, "p": 0.0}

        def stderr(line: str) -> None:
            match = _DETECTED.search(line)
            if match:
                found["language"] = normalize_language(match.group(1))
                try:
                    found["p"] = float(match.group(2))
                except ValueError:
                    found["p"] = 0.0

        try:
            extract_chunk(config, audio, start, length, wav, state)
            run_process(
                [
                    config.whisper,
                    "-m",
                    config.model,
                    "-f",
                    wav,
                    "-l",
                    "auto",
                    "-dl",
                    "-t",
                    str(config.threads),
                    "-ng",
                ],
                timeout=max(120.0, (length / 1000.0) * 4.0),
                state=state,
                on_stderr=stderr,
            )
        except EngineFailure:
            continue
        finally:
            try:
                os.unlink(wav)
            except OSError:
                pass
        if found["language"] and found["p"] >= DETECT_MIN_PROBABILITY:
            return found["language"]
    return None


def download_audio(client: Client, config: Config, claim: dict, state: JobState,
                   dest: str) -> str:
    episode_id = str(claim["episode_id"])
    lease = str(claim["lease"])
    url = client.url(str(claim.get("audio_url") or ""))
    raw_size = int(claim.get("size") or 0)
    expected = raw_size if raw_size > 0 else None
    if expected and expected > MAX_AUDIO_BYTES:
        raise EngineFailure("too_big", "l'audio de l'épisode dépasse 1 Gio")
    offset = os.path.getsize(dest) if os.path.exists(dest) else 0
    total = expected
    requests_made = 0
    while True:
        if state.stopping():
            raise Cancelled()
        requests_made += 1
        if requests_made > 20:
            raise EngineFailure("audio_http", "trop de reprises de téléchargement")
        offset = os.path.getsize(dest) if os.path.exists(dest) else 0
        headers = client._headers(lease)
        headers["Range"] = f"bytes={offset}-"
        try:
            response = client.session.get(
                url,
                headers=headers,
                stream=True,
                timeout=(10, config.audio_timeout),
                allow_redirects=False,
            )
        except requests.RequestException as exc:
            if requests_made >= config.retry_attempts:
                raise EngineFailure("audio_http", type(exc).__name__) from exc
            client._backoff(requests_made, None)
            continue
        try:
            status = response.status_code
            if status in (401, 403):
                payload = {}
                try:
                    candidate = response.json()
                    payload = candidate if isinstance(candidate, dict) else {}
                except ValueError:
                    payload = {}
                code = str(payload.get("code") or "")
                if status == 403 and code == "lease_expired":
                    raise HttpError(409, payload)
                raise AuthError(
                    status,
                    code or "unauthorized",
                    str(payload.get("error") or "audio refusé pour ce lease"),
                )
            if status in (404, 410):
                raise EngineFailure("not_archived", "l'audio de l'épisode est absent")
            if status == 416:
                match = re.match(r"bytes\s+\*/(\d+)", response.headers.get("Content-Range") or "")
                if match and offset == int(match.group(1)):
                    return dest
                offset = 0
                try:
                    os.unlink(dest)
                except OSError:
                    pass
                continue
            if status == 200:
                offset = 0
                total = int(response.headers.get("Content-Length") or 0) or total
                mode = "wb"
                end = None
            elif status == 206:
                match = _CONTENT_RANGE.match(response.headers.get("Content-Range") or "")
                if not match or int(match.group(1)) != offset:
                    raise EngineFailure("audio_http", "réponse Range incohérente")
                if match.group(3) != "*":
                    total = int(match.group(3))
                end = int(match.group(2))
                mode = "ab"
            elif status == 429 or 500 <= status < 600:
                if requests_made >= config.retry_attempts:
                    raise EngineFailure("audio_http", f"HTTP {status}")
                client._backoff(requests_made, response.headers.get("Retry-After"))
                continue
            else:
                raise EngineFailure("audio_http", f"HTTP {status}")
            if total and total > MAX_AUDIO_BYTES:
                raise EngineFailure("too_big", "l'audio de l'épisode dépasse 1 Gio")
            try:
                with open(dest, mode) as handle:
                    for block in response.iter_content(1024 * 1024):
                        if state.stopping():
                            raise Cancelled()
                        if block:
                            handle.write(block)
                            offset += len(block)
                            if offset > MAX_AUDIO_BYTES:
                                raise EngineFailure(
                                    "too_big", "l'audio de l'épisode dépasse 1 Gio"
                                )
            except OSError as exc:
                raise EngineFailure(
                    "no_space", "écriture temporaire impossible"
                ) from exc
            if end is not None and offset < end + 1:
                client._backoff(requests_made, None)
                continue
            if total and offset < total:
                client._backoff(requests_made, None)
                continue
            if total and offset > total:
                raise EngineFailure("audio_http", "audio plus long qu'annoncé")
            LOG.info(
                "épisode %s: audio téléchargé (%d octets, %d requête(s))",
                episode_id,
                offset,
                requests_made,
            )
            return dest
        except requests.RequestException as exc:
            if requests_made >= config.retry_attempts:
                raise EngineFailure("audio_http", type(exc).__name__) from exc
            client._backoff(requests_made, None)
            continue
        finally:
            response.close()


def wait_while_paused(state: JobState) -> None:
    while state.pause.is_set() and not state.stopping():
        time.sleep(0.5)


def sweep_temp(root: str, max_age: float = TEMP_MAX_AGE) -> int:
    moment = time.time()
    removed = 0
    try:
        entries = list(os.scandir(root))
    except OSError:
        return 0
    for entry in entries:
        try:
            if not entry.is_dir(follow_symlinks=False):
                continue
            if moment - entry.stat().st_mtime < max_age:
                continue
            shutil.rmtree(entry.path, ignore_errors=True)
            removed += 1
        except OSError:
            continue
    return removed


def ensure_temp_root(root: str) -> str:
    os.makedirs(root, mode=0o700, exist_ok=True)
    return root


def _fail(client: Client, episode_id: str, lease: str, code: str, message: str) -> None:
    LOG.error("épisode %s: %s: %s", episode_id, code, message)
    try:
        client.fail(episode_id, lease, code, message)
    except (AuthError, HttpError) as exc:
        LOG.warning("échec non signalé au serveur: %s", exc)


def _release(client: Client, episode_id: str, lease: str) -> None:
    try:
        client.release(episode_id, lease)
    except (AuthError, HttpError):
        pass


def run_job(config: Config, client: Client, claim: dict, state: JobState,
            workdir: str, engine_version: str) -> bool:
    episode_id = str(claim["episode_id"])
    lease = str(claim["lease"])
    duration_ms = int(claim.get("duration_ms") or 0)
    done_ms = max(0, int(claim.get("start_ms") or 0))
    chunk_ms = int(claim.get("chunk_ms") or config.chunk_seconds * 1000)
    overlap_ms = int(claim.get("overlap_ms") or OVERLAP_SECONDS * 1000)
    if chunk_ms != config.chunk_seconds * 1000:
        LOG.warning(
            "CHUNK_SECONDS=%s mais le serveur annonce %s ms ; le serveur fait foi",
            config.chunk_seconds,
            chunk_ms,
        )
    if duration_ms <= 0:
        _fail(client, episode_id, lease, "no_duration", "durée de l'épisode inconnue")
        return False
    language = config.language
    if language == "auto":
        language = normalize_language(claim.get("language")) or "auto"
    audio = os.path.join(workdir, "audio")
    try:
        download_audio(client, config, claim, state, audio)
        if language == "auto":
            detected = detect_language(config, audio, workdir, duration_ms, state)
            if detected:
                language = detected
                LOG.info("épisode %s: langue détectée %s", episode_id, language)
        LOG.info(
            "épisode %s: %d s d'audio, reprise à %d ms, langue %s",
            episode_id,
            duration_ms // 1000,
            done_ms,
            language,
        )
        while True:
            if state.stopping():
                raise Cancelled()
            if done_ms >= duration_ms:
                try:
                    result = client.complete(
                        episode_id,
                        lease,
                        {
                            "language": language,
                            "engine": ENGINE,
                            "model": os.path.basename(config.model),
                            "engine_version": engine_version,
                        },
                    )
                except HttpError as exc:
                    if exc.status == 409 and exc.code == "incomplete":
                        remaining = int(exc.payload.get("done_ms") or done_ms)
                        if remaining <= done_ms:
                            _fail(
                                client,
                                episode_id,
                                lease,
                                "incomplete",
                                "le serveur n'avance plus",
                            )
                            return False
                        done_ms = remaining
                        continue
                    if exc.status == 409 and exc.code in (
                        "lease_expired",
                        "lease_required",
                    ):
                        LOG.warning("épisode %s: lease expiré (complete)", episode_id)
                        state.lost.set()
                        return False
                    raise
                if result.get("status") == "ready":
                    LOG.info("épisode %s: transcription prête", episode_id)
                    return False
                if result.get("status"):
                    _fail(client, episode_id, lease, "complete", str(result))
                    return False
                return False
            wait_while_paused(state)
            length_ms = min(chunk_ms, duration_ms - done_ms)
            wav = os.path.join(workdir, f"chunk-{done_ms}.wav")
            prefix = os.path.join(workdir, f"chunk-{done_ms}")
            try:
                extract_chunk(config, audio, done_ms, length_ms, wav, state)
                state.progress(0)
                items, detected = recognize(
                    config, wav, prefix, language, length_ms, state
                )
                if language == "auto" and detected:
                    language = detected
                    LOG.info("épisode %s: langue %s", episode_id, language)
                segments = fit_segments(
                    normalize_segments(items, done_ms, length_ms, overlap_ms)
                )
                body = {
                    "start_ms": done_ms,
                    "duration_ms": length_ms,
                    "language": language,
                    "engine": ENGINE,
                    "model": os.path.basename(config.model),
                    "engine_version": engine_version,
                    "segments": segments,
                }
                try:
                    result = client.chunk(episode_id, lease, body)
                except HttpError as exc:
                    if exc.status == 409 and exc.code in (
                        "lease_expired",
                        "lease_required",
                    ):
                        LOG.warning("épisode %s: lease expiré (chunk)", episode_id)
                        state.lost.set()
                        return False
                    if exc.status == 409 and exc.code == "chunk_out_of_order":
                        LOG.warning(
                            "épisode %s: le serveur a déjà avancé ; reprise au prochain claim",
                            episode_id,
                        )
                        _release(client, episode_id, lease)
                        return False
                    if exc.status == 409 and exc.code == "too_many_segments":
                        _fail(
                            client,
                            episode_id,
                            lease,
                            "too_many_segments",
                            "trop de segments dans un chunk",
                        )
                        return False
                    raise
                previous = done_ms
                done_ms = int(result.get("done_ms") or 0)
                if done_ms <= previous:
                    LOG.warning(
                        "épisode %s: aucun progrès (done_ms %d), reprise au prochain claim",
                        episode_id,
                        done_ms,
                    )
                    _release(client, episode_id, lease)
                    return False
                LOG.info(
                    "épisode %s: chunk %d-%d ms, %d segment(s), done %d/%d ms",
                    episode_id,
                    previous,
                    previous + length_ms,
                    len(segments),
                    done_ms,
                    duration_ms,
                )
            finally:
                for leftover in (wav, prefix + ".json"):
                    try:
                        os.unlink(leftover)
                    except OSError:
                        pass
    except Cancelled:
        if not state.revoked.is_set():
            _release(client, episode_id, lease)
        return False
    except AuthError:
        state.revoked.set()
        LOG.error(
            "épisode %s: jeton ou appareil refusé ; recréez un appareil et son jeton",
            episode_id,
        )
        return True
    except EngineFailure as exc:
        _fail(client, episode_id, lease, exc.code, exc.message)
        return False
    except HttpError as exc:
        if exc.status == 409 and exc.code in ("lease_expired", "lease_required"):
            LOG.warning("épisode %s: lease expiré", episode_id)
            state.lost.set()
            return False
        _fail(client, episode_id, lease, "exit", f"serveur: {exc.message}")
        return False
    except Exception as exc:
        LOG.exception("épisode %s: erreur interne du worker", episode_id)
        _fail(
            client,
            episode_id,
            lease,
            "exit",
            f"erreur interne du worker ({type(exc).__name__})",
        )
        return False


def process_claim(config: Config, client: Client, claim: dict, engine_version: str,
                  stop: threading.Event) -> bool:
    episode_id = str(claim["episode_id"])
    lease = str(claim["lease"])
    if not _UUID.match(episode_id):
        LOG.error("claim invalide: identifiant d'épisode")
        return False
    state = JobState(stop)
    heartbeat_client = Client(config)
    heartbeat = Heartbeat(
        heartbeat_client, episode_id, lease, state, config.heartbeat_seconds
    )
    try:
        workdir = tempfile.mkdtemp(
            prefix=f"{episode_id[:8]}-", dir=ensure_temp_root(config.tmp_root)
        )
    except OSError:
        heartbeat_client.close()
        _fail(
            client,
            episode_id,
            lease,
            "no_space",
            "impossible de créer le dossier temporaire",
        )
        return False
    try:
        heartbeat.start()
    except RuntimeError:
        shutil.rmtree(workdir, ignore_errors=True)
        heartbeat_client.close()
        _fail(client, episode_id, lease, "exit", "heartbeat indisponible")
        return False
    try:
        return run_job(
            config, client, claim, state, workdir, engine_version
        )
    finally:
        heartbeat.stop()
        heartbeat.join(timeout=2.0)
        shutil.rmtree(workdir, ignore_errors=True)


def run(config: Config, engine_version: str, once: bool = False) -> int:
    stop = threading.Event()

    def on_signal(number, _frame):
        LOG.info("signal %d: arrêt demandé", number)
        stop.set()

    signal.signal(signal.SIGTERM, on_signal)
    signal.signal(signal.SIGINT, on_signal)
    sweep_temp(config.tmp_root)
    client = Client(config)
    try:
        while not stop.is_set():
            try:
                claim = client.claim()
            except AuthError:
                LOG.error("appareil refusé ou révoqué ; arrêt")
                return 2
            except HttpError as exc:
                LOG.warning("claim: %s", exc)
                stop.wait(config.poll_seconds)
                continue
            if claim is None:
                if once:
                    return 0
                stop.wait(config.poll_seconds)
                continue
            try:
                fatal = process_claim(config, client, claim, engine_version, stop)
            except Exception:
                LOG.exception("le traitement du job a échoué")
                fatal = False
            if fatal:
                return 2
            if once:
                return 0
        return 0
    finally:
        client.close()


def main(argv: Optional[list[str]] = None) -> int:
    parser = argparse.ArgumentParser(description="Neezer transcription sidecar")
    parser.add_argument(
        "--check",
        action="store_true",
        help="validate the model, the binaries and the configuration locally",
    )
    parser.add_argument(
        "--once",
        action="store_true",
        help="claim at most one job, then exit",
    )
    args = parser.parse_args(argv)
    try:
        config = Config.from_env()
    except ConfigError as exc:
        print(f"configuration: {exc}", file=sys.stderr)
        return 2
    logging.basicConfig(
        level=getattr(logging, config.log_level, logging.INFO),
        stream=sys.stderr,
        format="%(asctime)s %(levelname)s %(name)s: %(message)s",
    )
    errors, version = preflight(config)
    if args.check:
        if errors:
            for code, message in errors:
                print(f"{code}: {message}", file=sys.stderr)
            return 1
        try:
            size = os.path.getsize(config.model)
        except OSError:
            size = 0
        print(f"ok model {config.model} ({size} octets)")
        print(f"ok engine {config.whisper} ({version or 'inconnu'})")
        print(f"ok ffmpeg {config.ffmpeg}")
        print(f"ok url {config.url}")
        print(f"ok token {config.token_source}")
        return 0
    if errors:
        for code, message in errors:
            LOG.error("%s: %s", code, message)
        return 2
    LOG.info(
        "worker prêt: %s, modèle %s (%d octets), %d thread(s), chunk %d s, langue %s",
        config.url,
        os.path.basename(config.model),
        os.path.getsize(config.model),
        config.threads,
        config.chunk_seconds,
        config.language,
    )
    return run(config, version or "", once=args.once)


if __name__ == "__main__":
    raise SystemExit(main())
