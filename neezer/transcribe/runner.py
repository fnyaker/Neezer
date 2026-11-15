# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the AGPLv3 license.

"""Where transcription actually runs.

``JobQueue`` is the contract: claim a queued episode, work it, report progress,
complete or fail it, and release or expire the lease. The **inline runner** below
is its in-process deployment for a bare install and for tests. The separate
``neezer-transcriber`` container speaks the same HTTP lease with
``ExternalRunner`` and will implement ``claim``/``heartbeat``/``/chunk`` without
changing the queue or the engine; until that lot lands, an ``external`` runner
keeps jobs queued and says ``waiting`` — it never runs whisper in the API.

The inline runner is one thread, one episode: it reindexes itself in ``BULK``
(``renice`` + idle I/O), waits on ``workload.Throttle`` before a chunk, and
``SIGSTOP``s the whisper process group while a person has a stream open, which
is what keeps a three-hour episode from competing with a tapped track.
"""

from __future__ import annotations

import logging
import os
import shutil
import signal
import tempfile
import threading
import time

from ..db import PodcastTranscript, db
from ..deezer import workload
from . import engine, job, settings

logger = logging.getLogger(__name__)

POLL_SECONDS = 5.0
#: The external runner's refill tick: how long a completion can wait before the
#: next job is queued. One light pass every minute, never a whisper process.
EXTERNAL_POLL_SECONDS = 60.0
ASR_DIR = "asr"
TEMP_MAX_AGE = 24 * 3600
GATE_POLL = 2.0
GATE_CLEAR_SECONDS = 15.0

#: Gunicorn's worker count, as ``docker/gunicorn.conf.py`` reads it from the
#: same variable. The inline runner is one thread on one process: under N
#: workers each one would requeue what a sibling is transcribing (its boot
#: ``resume_if_interrupted``), and N whisper models is N times the memory.
WORKERS_ENV = "GUNICORN_WORKERS"
DEFAULT_WORKERS = 1


class _Shutdown(Exception):
    pass


class _CancelWatch:
    """The user's stop, read from the database at most twice a second."""

    def __init__(self, episode_id, interval=2.0):
        self.episode_id = str(episode_id)
        self.interval = interval
        self._at = 0.0
        self._value = False

    def is_set(self):
        if self._value:
            return True
        now_monotonic = time.monotonic()
        if now_monotonic - self._at < self.interval:
            return self._value
        self._at = now_monotonic
        row = (
            PodcastTranscript.select(PodcastTranscript.cancel_requested)
            .where(PodcastTranscript.episode == self.episode_id)
            .first()
        )
        self._value = bool(row and row.cancel_requested)
        return self._value


class _StopWhenEither:
    def __init__(self, *signals):
        self.signals = signals

    def is_set(self):
        return any(signal.is_set() for signal in self.signals)


class ProcessGate:
    """SIGSTOP the current chunk while somebody listens; SIGCONT when quiet."""

    def __init__(self, busy=None, poll=GATE_POLL, clear_for=GATE_CLEAR_SECONDS):
        self._busy = busy or _busy
        self.poll = poll
        self.clear_for = clear_for
        self.pid = None
        self.paused = False
        self._clear_since = None

    def attach(self, proc):
        self.pid = getattr(proc, "pid", None)
        self.paused = False
        self._clear_since = None

    def detach(self):
        if self.paused and self.pid and hasattr(signal, "SIGCONT"):
            self._signal(signal.SIGCONT)
        self.paused = False
        self._clear_since = None
        self.pid = None

    def _signal(self, number):
        try:
            os.killpg(os.getpgid(self.pid), number)
        except (OSError, AttributeError):
            pass

    def tick(self):
        if not self.pid or not hasattr(signal, "SIGSTOP"):
            return
        if self._busy():
            self._clear_since = None
            if not self.paused:
                self._signal(signal.SIGSTOP)
                self.paused = True
            return
        if not self.paused:
            return
        moment = time.monotonic()
        if self._clear_since is None:
            self._clear_since = moment
        elif moment - self._clear_since >= self.clear_for:
            self._signal(signal.SIGCONT)
            self.paused = False
            self._clear_since = None


def _busy():
    return (
        workload.streams_active() > 0
        or workload.foreground_busy()
        or workload.listening_for() < 15
        or workload.load_per_core() > 0.8
    )


def _testing(app):
    if app is None:
        return True
    tested = getattr(app, "testing", None)
    if tested is None:
        tested = getattr(app, "TESTING", None)
    if tested is not None:
        return bool(tested)
    return bool((getattr(app, "config", None) or {}).get("TESTING"))


def workers():
    """How many server processes this deployment runs (1 when it says nothing)."""
    raw = os.environ.get(WORKERS_ENV, str(DEFAULT_WORKERS))
    try:
        return max(1, int(str(raw).strip()))
    except (TypeError, ValueError):
        return DEFAULT_WORKERS


def inline_blocked(app=None):
    """Why this process must not run the inline runner, or None.

    ``GUNICORN_WORKERS > 1`` is the one case: the external sidecar is then the
    only supported deployment for transcription.
    """
    if settings.runner(app) != "inline":
        return None
    return "multi_worker" if workers() > 1 else None


class JobQueue:
    """What the API promises a runner; the inline one is in this process."""

    name = "inline"

    def start(self):
        raise NotImplementedError

    def stop(self):
        raise NotImplementedError

    def wake(self):
        raise NotImplementedError

    def status(self):
        raise NotImplementedError

    def drain(self, limit=None):
        raise NotImplementedError


class InlineRunner(JobQueue):
    name = "inline"

    def __init__(self, app=None, poll=POLL_SECONDS):
        self.app = app
        self.poll = poll
        self._wake = threading.Event()
        self._stop = threading.Event()
        self._thread = None
        self._reason = None
        self._active = None
        self._lock = threading.Lock()

    def start(self):
        if self._thread is not None:
            return self
        blocked = inline_blocked(self.app)
        if blocked:
            self._reason = blocked
            logger.warning(
                "transcription: runner=inline cannot serve %d Gunicorn workers — "
                "each worker would requeue the jobs its siblings are transcribing "
                "and hold its own copy of the model. Set runner = external in the "
                "[transcribe] section (a sidecar claims the jobs) or run one "
                "worker; jobs stay queued meanwhile, they are not lost.",
                workers(),
            )
            return self
        if _testing(self.app):
            return self
        self._thread = threading.Thread(
            target=self._loop, name="transcribe-inline", daemon=True
        )
        self._thread.start()
        return self

    def stop(self, timeout=10.0):
        self._stop.set()
        self._wake.set()
        thread = self._thread
        if thread is not None and thread is not threading.current_thread():
            thread.join(timeout)

    def wake(self):
        self._wake.set()

    def status(self):
        with self._lock:
            return {
                "runner": self.name,
                "started": self._thread is not None,
                "reason": self._reason,
                "active": self._active,
            }

    def _loop(self):
        try:
            db.connect(reuse_if_open=True)
        except Exception:
            logger.debug("transcription runner could not open the database", exc_info=True)
        workload.renice(15)
        self.sweep_temp()
        try:
            job.resume_if_interrupted(self.app)
        except Exception:
            logger.exception("could not resume interrupted transcriptions")
        while not self._stop.is_set():
            try:
                self._wake.wait(self.poll)
                self._wake.clear()
                if self._stop.is_set():
                    break
                self.run_once()
            except Exception:
                logger.exception("transcription runner loop failed")
                time.sleep(self.poll)

    def run_once(self):
        values = settings.load(self.app)
        if not values["enabled"]:
            self._reason = "disabled"
            return None
        verdict = engine.quick_check(settings.paths(self.app))
        if not verdict["ok"]:
            self._reason = verdict["reason"]
            return None
        self._reason = None
        transcript = job.claim_inline(self.app)
        if transcript is None:
            job.resume_auto(self.app)
            return None
        self._run(transcript)
        job.resume_auto(self.app)
        return transcript

    def drain(self, limit=None):
        done = 0
        while limit is None or done < limit:
            transcript = self.run_once()
            if transcript is None:
                break
            done += 1
        return done

    def _run(self, transcript):
        episode_id = str(transcript.episode_id)
        with self._lock:
            self._active = episode_id
        job.note_progress(episode_id, transcript.done_ms or 0, 0, 0)
        values = settings.load(self.app)
        paths = settings.paths(self.app)
        verdict = engine.probe(paths)
        if not verdict["ok"]:
            job.mark_error(episode_id, verdict["reason"], verdict["message"])
            self._finish(episode_id)
            return
        model = engine.choose_model(paths["model"], engine.free_memory_bytes())
        if not model:
            job.mark_error(episode_id, "model_missing", "aucun modèle utilisable")
            self._finish(episode_id)
            return
        paths = dict(paths, model=model)
        episode = transcript.episode
        try:
            audio = job.archive_file(episode, paths["root"])
        except job.QueueError as exc:
            job.mark_error(episode_id, exc.code, exc.message)
            self._finish(episode_id)
            return
        work_dir = tempfile.mkdtemp(
            prefix=f"{episode_id[:8]}-",
            dir=self._asr_dir(),
        )
        cancel = _CancelWatch(episode_id)
        stop_when = _StopWhenEither(cancel, self._stop)
        gate = ProcessGate()
        language = job.resolve_language(transcript, episode, values)
        try:
            if language == "auto":
                detected, _probability = engine.detect_language(
                    paths,
                    audio,
                    work_dir,
                    transcript.duration_ms,
                    settings.threads(),
                    cancel=stop_when,
                    on_tick=gate.tick,
                    on_start=gate.attach,
                    paused=lambda: gate.paused,
                )
                if detected:
                    language = detected
                    job.set_outcome(episode_id, language=detected)
            self._work(
                transcript,
                paths,
                audio,
                work_dir,
                language,
                model,
                verdict.get("version"),
                cancel,
                stop_when,
                gate,
            )
        except engine.Cancelled:
            if self._stop.is_set() and not cancel.is_set():
                return
            job.flush_sidecar(episode_id, self.app)
            job.mark_stopped(episode_id)
        except _Shutdown:
            return
        except engine.EngineError as exc:
            job.flush_sidecar(episode_id, self.app)
            job.mark_error(episode_id, exc.code, str(exc))
            logger.info("Transcription of %s failed: %s", episode_id, exc)
        except job.QueueError as exc:
            job.flush_sidecar(episode_id, self.app)
            job.mark_error(episode_id, exc.code, exc.message)
        except Exception as exc:
            job.flush_sidecar(episode_id, self.app)
            job.mark_error(episode_id, "exit", "erreur interne du runner")
            logger.exception("Transcription of %s crashed: %s", episode_id, exc)
        finally:
            gate.detach()
            shutil.rmtree(work_dir, ignore_errors=True)
            self._finish(episode_id)

    def _work(self, transcript, paths, audio, work_dir, language, model,
              version, cancel, stop_when, gate):
        episode_id = str(transcript.episode_id)
        duration = int(transcript.duration_ms or 0)
        done = int(transcript.done_ms or 0)
        thread_count = settings.threads()
        while done < duration:
            if cancel.is_set():
                raise engine.Cancelled()
            if self._stop.is_set():
                raise _Shutdown()
            throttle = workload.Throttle(
                duty=1.0,
                listen_quiet=15.0,
                max_load=0.8,
                stop=lambda: stop_when.is_set(),
                poll=GATE_POLL,
            )
            if not throttle.wait():
                if self._stop.is_set():
                    raise _Shutdown()
                raise engine.Cancelled()
            length = min(job.CHUNK_MS, duration - done)
            if length <= 0:
                break
            wav = os.path.join(work_dir, f"chunk-{done}.wav")
            prefix = os.path.join(work_dir, f"chunk-{done}")
            try:
                engine.extract_chunk(
                    paths,
                    audio,
                    done,
                    length,
                    wav,
                    cancel=stop_when,
                    on_tick=gate.tick,
                    on_start=gate.attach,
                    paused=lambda: gate.paused,
                )
                raw, detected = engine.recognize(
                    paths,
                    wav,
                    prefix,
                    language,
                    thread_count,
                    length,
                    cancel=stop_when,
                    on_progress=lambda percent: job.note_progress(
                        episode_id, done, length, percent
                    ),
                    on_tick=gate.tick,
                    on_start=gate.attach,
                    paused=lambda: gate.paused,
                )
            finally:
                gate.detach()
            if language == "auto" and detected:
                language = detected
                job.set_outcome(episode_id, language=detected)
            segments = engine.normalize_segments(
                raw, done, length, job.OVERLAP_MS
            )
            result = job.append_inline(
                episode_id,
                done,
                length,
                segments,
                language=language,
                model=os.path.basename(model),
                engine_version=(version or "")[:32] or None,
            )
            done = int(result["done_ms"])
            job.flush_sidecar(episode_id, self.app)
            for leftover in (wav, prefix + ".json"):
                try:
                    os.unlink(leftover)
                except OSError:
                    pass
        if done < duration - 5000:
            raise job.QueueError("incomplete", "la transcription s'est arrêtée avant la fin")
        path = job.flush_sidecar(episode_id, self.app)
        job.mark_ready(
            episode_id,
            transcript_path=path,
            language=None if language == "auto" else language,
            model=os.path.basename(model),
            engine_version=(version or "")[:32] or None,
        )

    def _finish(self, episode_id):
        job.clear_progress(episode_id)
        with self._lock:
            self._active = None

    def _asr_dir(self):
        root = os.path.join(settings.cache_dir(self.app), ASR_DIR)
        os.makedirs(root, exist_ok=True)
        return root

    def sweep_temp(self, max_age=TEMP_MAX_AGE):
        """Remove chunk directories a killed runner left behind."""
        root = self._asr_dir()
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


class ExternalRunner(JobQueue):
    """The sidecar container's contract: claim, heartbeat, chunk, complete.

    It never runs whisper in the API process. It does keep the *queue* moving:
    a light scheduler thread (one pass a minute, reniced, BULK) requeues what a
    restart left ``running``, expires lost leases and refills the free slots
    from the marked shows. Without it the backlog stopped at its first batch
    for ever — ``resume_auto`` had no other caller outside the inline runner.
    """

    name = "external"

    def __init__(self, app=None, poll=EXTERNAL_POLL_SECONDS):
        self.app = app
        self.poll = poll
        self.reason = "external_runner_not_started"
        self._stop = threading.Event()
        self._wake = threading.Event()
        self._thread = None

    def start(self):
        if self._thread is not None:
            return self
        if _testing(self.app):
            return self
        self._thread = threading.Thread(
            target=self._loop, name="transcribe-scheduler", daemon=True
        )
        self._thread.start()
        return self

    def stop(self, timeout=5.0):
        self._stop.set()
        self._wake.set()
        thread = self._thread
        if thread is not None and thread is not threading.current_thread():
            thread.join(timeout)

    def wake(self):
        self._wake.set()

    def status(self):
        return {
            "runner": self.name,
            "started": self._thread is not None,
            "reason": self.reason,
            "active": None,
        }

    def resume(self):
        """Requeue what a restart interrupted, then fill the free slots once."""
        return job.resume_if_interrupted(self.app)

    def refill(self):
        """Fill the free queue slots from the marked shows; runs no engine."""
        return job.resume_auto(self.app)

    def _loop(self):
        try:
            db.connect(reuse_if_open=True)
        except Exception:
            logger.debug(
                "transcription scheduler could not open the database", exc_info=True
            )
        workload.renice(15)
        try:
            self.resume()
        except Exception:
            logger.exception("could not resume interrupted transcriptions")
        self.reason = None
        while not self._stop.is_set():
            try:
                self.refill()
            except Exception:
                logger.exception("transcription scheduler failed")
            self._wake.wait(self.poll)
            self._wake.clear()

    def drain(self, limit=None):
        return 0


_runner_lock = threading.Lock()
_runner = None


def build_runner(app=None):
    if settings.runner(app) == "external":
        return ExternalRunner(app)
    return InlineRunner(app)


def start(app=None):
    global _runner
    with _runner_lock:
        if _runner is not None:
            return _runner
        runner = build_runner(app)
        _runner = runner
    runner.start()
    return runner


def stop():
    global _runner
    with _runner_lock:
        runner = _runner
        _runner = None
    if runner is not None:
        runner.stop()
    return runner


def wake():
    with _runner_lock:
        runner = _runner
    if runner is not None:
        runner.wake()


def status():
    with _runner_lock:
        runner = _runner
    if runner is None:
        values = settings.load(None)
        return {
            "runner": values["runner"],
            "started": False,
            "reason": "not_started",
            "active": None,
        }
    return runner.status()


def drain(app=None, limit=None):
    """Run queued jobs synchronously on this thread (tests and the CLI's tests)."""
    runner = InlineRunner(app)
    try:
        db.connect(reuse_if_open=True)
    except Exception:
        pass
    return runner.drain(limit=limit)


def sweep_temp(app=None):
    return InlineRunner(app).sweep_temp()
