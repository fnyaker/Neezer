# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""whisper.cpp, on this machine's CPU, one chunk of audio at a time.

Nothing here reads the database and nothing here knows Flask: the same code runs
on the inline runner, inside the sidecar container and, later, on an enrolled
client. The archive file is the only input, the model and the binary are paths
from the configuration file (never from a request), and a model is never
downloaded here.

A chunk is ``CHUNK_SECONDS`` of 16 kHz mono WAV with ``OVERLAP_SECONDS`` shared
with the previous one. Segments are converted to absolute milliseconds and the
slice fully inside the previous chunk's overlap is dropped, so a resumed run
neither repeats ten minutes nor loses a word at the seam.
"""

from __future__ import annotations

import collections
import json
import logging
import math
import os
import re
import shutil
import signal
import subprocess
import sys
import threading
import time

logger = logging.getLogger(__name__)

FORMAT_ENGINE = "whisper.cpp"
CHUNK_SECONDS = 600
OVERLAP_SECONDS = 2
SAMPLE_RATE = 16000
MAX_SEGMENTS_PER_CHUNK = 2000
MAX_TEXT = 2000
DETECT_SECONDS = 120
DETECT_MIN_PROBABILITY = 0.6

MODEL_TURBO_Q8 = "ggml-large-v3-turbo-q8_0.bin"
MODEL_TURBO_Q5 = "ggml-large-v3-turbo-q5_0.bin"
MODEL_SMALL = "ggml-small.bin"
MODEL_ORDER_FREE = (MODEL_TURBO_Q8, MODEL_TURBO_Q5, MODEL_SMALL)
MODEL_ORDER_TIGHT = (MODEL_TURBO_Q5, MODEL_SMALL)
TURBO_FREE_BYTES = 2_500_000_000
MIN_MODEL_BYTES = 1 << 20
MODEL_MAGICS = (b"ggml", b"lmgg")

PROBE_TTL = 60.0

WHISPER_LANGUAGES = frozenset(
    {
        "auto", "af", "am", "ar", "as", "az", "ba", "be", "bg", "bn", "bo", "br",
        "bs", "ca", "cs", "cy", "da", "de", "el", "en", "es", "et", "eu", "fa",
        "fi", "fo", "fr", "gl", "gu", "ha", "haw", "he", "hi", "hr", "ht", "hu",
        "hy", "id", "is", "it", "ja", "jw", "ka", "kk", "km", "kn", "ko", "la",
        "lb", "ln", "lo", "lt", "lv", "mg", "mi", "mk", "ml", "mn", "mr", "ms",
        "mt", "my", "ne", "nl", "nn", "no", "oc", "pa", "pl", "ps", "pt", "ro",
        "ru", "sa", "sd", "si", "sk", "sl", "sn", "so", "sq", "sr", "su", "sv",
        "sw", "ta", "te", "tg", "th", "tk", "tl", "tr", "tt", "uk", "ur", "uz",
        "vi", "yi", "yo", "zh",
    }
)

_PROGRESS = re.compile(r"progress\s*=\s*(\d{1,3})\s*%")
_DETECTED = re.compile(
    r"auto-detected language:\s*([a-z]{2,3})\s*\(\s*p\s*=\s*([0-9.]+)\s*\)"
)
_TAG_ONLY = re.compile(r"^\[[^\]]*\]$")
_CONTROL = re.compile(r"[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]")
_WHITESPACE = re.compile(r"\s+")


class EngineError(RuntimeError):
    code = "exit"

    def __init__(self, message, code=None, detail=None):
        super().__init__(message)
        if code:
            self.code = code
        self.detail = detail or message


class Cancelled(EngineError):
    code = "cancelled"

    def __init__(self, message="transcription annulée"):
        super().__init__(message)


class TimedOut(EngineError):
    code = "timeout"

    def __init__(self, message="le moteur a dépassé son délai"):
        super().__init__(message)


class AudioError(EngineError):
    code = "no_audio"


class Cancel:
    """A stop flag the runner can set; the engine only ever reads it."""

    def __init__(self):
        self._event = threading.Event()

    def set(self):
        self._event.set()

    def is_set(self):
        return self._event.is_set()


def normalize_language(value):
    text = str(value or "").strip().lower()
    return text if text in WHISPER_LANGUAGES else None


def model_name(path):
    return os.path.basename(str(path)) if path else None


def _is_regular(path):
    try:
        return os.path.isfile(path)
    except (OSError, ValueError):
        return False


def _is_executable(path):
    try:
        return _is_regular(path) and os.access(path, os.X_OK)
    except (OSError, ValueError):
        return False


def valid_model(path):
    if not _is_regular(path):
        return False
    try:
        if os.path.getsize(path) < MIN_MODEL_BYTES:
            return False
        with open(path, "rb") as handle:
            magic = handle.read(4)
    except OSError:
        return False
    return magic in MODEL_MAGICS


def _check(paths):
    binary = paths.get("bin")
    model = paths.get("model")
    ffmpeg = paths.get("ffmpeg") or shutil.which("ffmpeg")
    if binary and _is_regular(binary) and not _is_executable(binary):
        return _failure("engine_missing", "le binaire whisper n'est pas exécutable")
    if not binary or not _is_executable(binary):
        return _failure("engine_missing", "moteur introuvable (whisper-cli)")
    if not model or not _is_regular(model):
        return _failure("model_missing", "modèle introuvable")
    if not valid_model(model):
        try:
            size = os.path.getsize(model)
        except OSError:
            size = 0
        if size and size < MIN_MODEL_BYTES:
            return _failure("model_invalid", "modèle trop petit")
        return _failure("model_invalid", "modèle invalide (magie GGML absente)")
    if not ffmpeg or (shutil.which(ffmpeg) is None and not _is_executable(ffmpeg)):
        return _failure("ffmpeg_missing", "ffmpeg introuvable")
    return None


def _failure(reason, message):
    return {"ok": False, "reason": reason, "message": message}


def quick_check(paths):
    """The cheap, subprocess-free verdict used before a row is queued."""
    failure = _check(paths)
    model = paths.get("model")
    base = {
        "ok": failure is None,
        "reason": failure["reason"] if failure else None,
        "message": failure["message"] if failure else None,
        "engine": FORMAT_ENGINE,
        "model": model_name(model),
        "model_bytes": 0,
        "version": None,
    }
    try:
        base["model_bytes"] = os.path.getsize(model) if model else 0
    except OSError:
        pass
    return base


_probe_lock = threading.Lock()
_probe_cache = {"key": None, "at": 0.0, "value": None}


def clear_probe_cache():
    with _probe_lock:
        _probe_cache.update(key=None, at=0.0, value=None)


def _probe_key(paths):
    pieces = []
    for name in ("bin", "model", "ffmpeg"):
        path = paths.get(name) or ""
        try:
            info = os.stat(path) if path else None
        except OSError:
            info = None
        pieces.append(
            (path, info.st_size if info else 0, info.st_mtime_ns if info else 0)
        )
    return tuple(pieces)


def probe(paths, ttl=PROBE_TTL):
    key = _probe_key(paths)
    now = time.monotonic()
    with _probe_lock:
        if (
            _probe_cache["value"] is not None
            and _probe_cache["key"] == key
            and now - _probe_cache["at"] < ttl
        ):
            return dict(_probe_cache["value"])
    result = quick_check(paths)
    if result["ok"]:
        try:
            proc = subprocess.run(
                [paths["bin"], "--version"],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                timeout=10,
            )
            text = (proc.stdout or b"").decode("utf-8", "replace").strip()
            if proc.returncode != 0:
                result = _failure("engine_error", "le binaire whisper ne répond pas")
            else:
                result["version"] = text.splitlines()[0][:32] if text else None
        except (OSError, subprocess.SubprocessError):
            result = _failure("engine_error", "le binaire whisper ne répond pas")
    with _probe_lock:
        _probe_cache.update(key=key, at=now, value=dict(result))
    return result


def free_memory_bytes():
    try:
        with open("/proc/meminfo", encoding="ascii") as handle:
            for line in handle:
                if line.startswith("MemAvailable:"):
                    return int(line.split()[1]) * 1024
    except (OSError, ValueError, IndexError):
        return None
    return None


def choose_model(model_path, free_bytes=None, order_free=MODEL_ORDER_FREE,
                 order_tight=MODEL_ORDER_TIGHT):
    """The best model already on disk for the memory this box has right now.

    The operator's ``model`` names the preferred file; its directory is scanned
    for the known quantisations, so a 1 GB VPS uses ``small`` without anyone
    editing the configuration. Unknown free memory keeps the configured file.
    """
    if not model_path:
        return None
    if free_bytes is None:
        return model_path if valid_model(model_path) else None
    directory = os.path.dirname(model_path) or "."
    order = order_free if free_bytes >= TURBO_FREE_BYTES else order_tight
    for name in order:
        candidate = os.path.join(directory, name)
        if valid_model(candidate):
            return candidate
    return model_path if valid_model(model_path) else None


def threads(cpus=None):
    if cpus is None:
        try:
            cpus = len(os.sched_getaffinity(0))
        except (AttributeError, OSError):
            cpus = os.cpu_count() or 1
    cpus = max(1, int(cpus))
    half = max(1, cpus // 2)
    return min(4, half) if cpus >= 16 else min(2, half)


def _preexec():
    if not sys.platform.startswith("linux"):
        return None

    def hook():
        try:
            import ctypes

            libc = ctypes.CDLL(None, use_errno=True)
            libc.prctl(1, signal.SIGKILL, 0, 0, 0)
        except Exception:
            pass

    return hook


def _terminate(proc):
    try:
        os.killpg(os.getpgid(proc.pid), signal.SIGCONT)
    except (OSError, AttributeError):
        pass
    try:
        os.killpg(os.getpgid(proc.pid), signal.SIGTERM)
    except (OSError, AttributeError):
        try:
            proc.terminate()
        except OSError:
            pass
    try:
        proc.wait(timeout=5)
        return
    except subprocess.TimeoutExpired:
        pass
    try:
        os.killpg(os.getpgid(proc.pid), signal.SIGKILL)
    except (OSError, AttributeError):
        try:
            proc.kill()
        except OSError:
            pass
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        pass


def run_process(cmd, timeout, cancel=None, on_stderr=None, on_tick=None,
                on_start=None, paused=None):
    proc = subprocess.Popen(
        cmd,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        start_new_session=True,
        preexec_fn=_preexec(),
    )
    tail = collections.deque(maxlen=20)

    def read_stderr():
        try:
            for raw in proc.stderr:
                line = raw.decode("utf-8", "replace").rstrip()
                tail.append(line)
                if on_stderr is not None:
                    try:
                        on_stderr(line)
                    except Exception:
                        logger.debug("stderr hook failed", exc_info=True)
        except (OSError, ValueError):
            pass

    if on_start is not None:
        try:
            on_start(proc)
        except Exception:
            logger.debug("process start hook failed", exc_info=True)
    reader = threading.Thread(target=read_stderr, daemon=True)
    reader.start()
    deadline = time.monotonic() + max(1.0, float(timeout))
    last = time.monotonic()
    code = None
    try:
        while True:
            code = proc.poll()
            if code is not None:
                break
            if cancel is not None and cancel.is_set():
                _terminate(proc)
                raise Cancelled()
            if on_tick is not None:
                try:
                    on_tick()
                except Exception:
                    logger.debug("process tick failed", exc_info=True)
            moment = time.monotonic()
            if paused is not None and moment > last:
                try:
                    if paused():
                        deadline += moment - last
                except Exception:
                    logger.debug("pause hook failed", exc_info=True)
            last = moment
            if time.monotonic() > deadline:
                _terminate(proc)
                raise TimedOut()
            time.sleep(0.2)
    finally:
        reader.join(1.0)
        try:
            proc.stderr.close()
        except (AttributeError, OSError):
            pass
    if code != 0:
        detail = tail[-1] if tail else f"code {code}"
        raise EngineError(f"le moteur s'est arrêté ({detail})"[:300], code="exit")
    return tail


def extract_chunk(paths, audio, start_ms, duration_ms, out_wav, cancel=None,
                  on_tick=None, on_start=None, paused=None):
    ffmpeg = paths.get("ffmpeg") or shutil.which("ffmpeg") or "ffmpeg"
    seconds = max(0.001, duration_ms / 1000.0)
    cmd = [
        ffmpeg,
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
        run_process(
            cmd,
            timeout=max(120.0, seconds * 2.0),
            cancel=cancel,
            on_tick=on_tick,
            on_start=on_start,
            paused=paused,
        )
    except Cancelled:
        raise
    except TimedOut:
        raise
    except EngineError as exc:
        raise AudioError(
            "l'audio de l'épisode n'a pas pu être lu", detail=str(exc)
        ) from exc
    if not _is_regular(out_wav) or os.path.getsize(out_wav) < 44:
        raise AudioError("l'extrait audio est vide")
    return out_wav


def parse_json(path):
    try:
        with open(path, encoding="utf-8") as handle:
            data = json.load(handle)
    except (OSError, ValueError) as exc:
        raise EngineError("le JSON du moteur est illisible", code="bad_json") from exc
    if not isinstance(data, dict):
        raise EngineError("le JSON du moteur n'a pas la bonne forme", code="bad_json")
    items = data.get("transcription")
    if not isinstance(items, list):
        raise EngineError("le JSON du moteur n'a pas de transcription", code="bad_json")
    language = data.get("result", {}).get("language") if isinstance(
        data.get("result"), dict
    ) else None
    language = normalize_language(language)
    return items, language


def recognize(paths, wav, out_prefix, language, thread_count, duration_ms,
              cancel=None, on_progress=None, on_tick=None, on_start=None,
              paused=None):
    language = normalize_language(language) or "auto"
    cmd = [
        paths["bin"],
        "-m",
        paths["model"],
        "-f",
        wav,
        "-oj",
        "-of",
        out_prefix,
        "-l",
        language,
        "-t",
        str(thread_count),
        "-ng",
        "-pp",
        "-sns",
    ]

    def stderr(line):
        match = _PROGRESS.search(line)
        if match and on_progress is not None:
            percent = max(0, min(100, int(match.group(1))))
            try:
                on_progress(percent)
            except Exception:
                logger.debug("progress hook failed", exc_info=True)

    run_process(
        cmd,
        timeout=max(900.0, (duration_ms / 1000.0) * 6.0),
        cancel=cancel,
        on_stderr=stderr,
        on_tick=on_tick,
        on_start=on_start,
        paused=paused,
    )
    return parse_json(out_prefix + ".json")


run_chunk = recognize


def detect_language(paths, audio, work_dir, duration_ms, thread_count,
                    cancel=None, on_tick=None, on_start=None, paused=None):
    """One detection for the whole episode, never one per 30 s window."""
    if duration_ms < 5000:
        return None, None
    windows = [0]
    if duration_ms > DETECT_SECONDS * 1000 * 2:
        windows.append(int(duration_ms * 0.25))
    for index, start_ms in enumerate(windows):
        wav = os.path.join(work_dir, f"detect-{index}.wav")
        length = min(DETECT_SECONDS * 1000, duration_ms - start_ms)
        try:
            extract_chunk(
                paths,
                audio,
                start_ms,
                length,
                wav,
                cancel=cancel,
                on_tick=on_tick,
                on_start=on_start,
                paused=paused,
            )
        except (Cancelled, TimedOut):
            raise
        except EngineError:
            return None, None
        found = {"language": None, "p": 0.0}

        def stderr(line):
            match = _DETECTED.search(line)
            if match:
                found["language"] = normalize_language(match.group(1))
                try:
                    found["p"] = float(match.group(2))
                except ValueError:
                    found["p"] = 0.0

        try:
            run_process(
                [
                    paths["bin"],
                    "-m",
                    paths["model"],
                    "-f",
                    wav,
                    "-l",
                    "auto",
                    "-dl",
                    "-t",
                    str(thread_count),
                    "-ng",
                ],
                timeout=max(120.0, (length / 1000.0) * 4.0),
                cancel=cancel,
                on_stderr=stderr,
                on_tick=on_tick,
                on_start=on_start,
                paused=paused,
            )
        except (Cancelled, TimedOut):
            raise
        except EngineError:
            return None, None
        try:
            os.unlink(wav)
        except OSError:
            pass
        if found["language"] and found["p"] >= DETECT_MIN_PROBABILITY:
            return found["language"], found["p"]
    return None, None


def _milliseconds(value):
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return None
    if not math.isfinite(value):
        return None
    return int(round(value))


def clean_text(raw):
    if not isinstance(raw, str):
        return None
    text = _CONTROL.sub(" ", raw)
    text = _WHITESPACE.sub(" ", text).strip()
    if not text or _TAG_ONLY.match(text):
        return None
    return text[:MAX_TEXT].strip() or None


def normalize_segments(items, chunk_start_ms, chunk_duration_ms,
                       overlap_ms=OVERLAP_SECONDS * 1000):
    """Raw whisper segments -> absolute, bounded, de-duplicated lines.

    The first ``overlap_ms`` after the chunk start belongs to the previous
    chunk, which already emitted it; segments ending before that point are
    dropped. Everything else is clamped to the chunk and stripped of tags,
    control characters and anything over ``MAX_TEXT``.
    """
    chunk_start_ms = max(0, int(chunk_start_ms))
    chunk_end_ms = chunk_start_ms + max(1, int(chunk_duration_ms))
    cutoff = chunk_start_ms + max(0, int(overlap_ms)) if chunk_start_ms > 0 else 0
    out = []
    for item in items or ():
        if len(out) >= MAX_SEGMENTS_PER_CHUNK:
            break
        if not isinstance(item, dict):
            continue
        offsets = item.get("offsets")
        if not isinstance(offsets, dict):
            continue
        start = _milliseconds(offsets.get("from"))
        end = _milliseconds(offsets.get("to"))
        if start is None or end is None:
            continue
        start = max(0, start) + chunk_start_ms
        end = max(0, end) + chunk_start_ms
        if end <= start:
            continue
        if cutoff and end <= cutoff:
            continue
        start = max(start, chunk_start_ms)
        end = min(end, chunk_end_ms)
        if end <= start:
            continue
        text = clean_text(item.get("text"))
        if not text:
            continue
        out.append({"start_ms": start, "end_ms": end, "text": text})
    out.sort(key=lambda segment: (segment["start_ms"], segment["end_ms"]))
    return out
