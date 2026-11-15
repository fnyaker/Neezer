# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the AGPLv3 license.

"""The queue, the states and the lease that make transcription resumable.

``queued -> leased | running -> ready`` and, on the way, ``stopped`` or
``error``. An external worker takes a lease (15 min, heartbeated); the inline
runner takes ``running`` through the same conditional UPDATE and holds a
process-level lock so only one episode runs at a time. Every transition is a
conditional write, so a double click, a second claimer or a restarted API cannot
duplicate work, and ``done_ms`` plus the stored segments are what a retry
resumes from.

The database is the index, not the text: the canonical copy lives in the
episode's ``.transcript.json`` sidecar (``sidecar.py``). Nothing here launches a
subprocess and nothing here runs inside a request: ``enqueue`` writes a row and
wakes the runner.
"""

from __future__ import annotations

import hashlib
import hmac
import logging
import os
import re
import secrets
import threading
import unicodedata
import uuid
from datetime import timedelta

from peewee import SqliteDatabase, fn

from ..db import (
    Meta,
    PodcastChannel,
    PodcastEpisode,
    PodcastTranscript,
    PodcastTranscriptLease,
    PodcastTranscriptSegment,
    TranscribeWorker,
    db,
    now,
)
from . import engine, settings, sidecar

logger = logging.getLogger(__name__)

QUEUE_MAX = 20
LEASE_SECONDS = 15 * 60
HEARTBEAT_WRITE_SECONDS = 60
LEASE_ATTEMPTS = 3
MAX_BYTES = 1 << 30
CHUNK_MS = engine.CHUNK_SECONDS * 1000
OVERLAP_MS = engine.OVERLAP_SECONDS * 1000
NOMINAL_XRT = 2.0

ACTIVE = ("queued", "leased", "running")
STATUSES = ("queued", "leased", "running", "ready", "error", "stopped")
SOURCES = ("server", "client")

_CONTROL = re.compile(r"[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]")
_SPACES = re.compile(r"\s+")
_TAG_ONLY = re.compile(r"^\[[^\]]*\]$")


class QueueError(RuntimeError):
    def __init__(self, code, message, detail=None, retry_in=None):
        super().__init__(message)
        self.code = code
        self.message = message
        self.detail = detail
        self.retry_in = retry_in


class LeaseError(QueueError):
    pass


def fold(text):
    value = unicodedata.normalize("NFD", (text or "").casefold())
    return "".join(char for char in value if not unicodedata.combining(char))


def episode_key(value):
    try:
        return str(uuid.UUID(str(value)))
    except (AttributeError, TypeError, ValueError) as exc:
        raise QueueError("invalid_id", "identifiant d'épisode invalide") from exc


def clean_text(raw):
    if not isinstance(raw, str):
        return None
    text = _SPACES.sub(" ", _CONTROL.sub(" ", raw)).strip()
    if not text or _TAG_ONLY.match(text):
        return None
    return text[: engine.MAX_TEXT].strip() or None


def audio_duration_ms(episode):
    seconds = int(episode.duration or 0)
    if seconds > 0:
        return seconds * 1000
    if not episode.path:
        return None
    try:
        from mutagen import File

        info = File(episode.path)
        length = float(info.info.length) if info is not None else 0.0
    except Exception:
        return None
    return int(round(length * 1000)) if length > 0 else None


def archive_file(episode, root):
    if not episode.path:
        raise QueueError("not_archived", "l'épisode n'est pas archivé sur le serveur")
    if not root:
        raise QueueError("no_archive", "aucune archive de podcasts n'est configurée")
    if not os.path.isfile(episode.path):
        raise QueueError("not_archived", "le fichier archivé de l'épisode est absent")
    if not sidecar.inside(episode.path, root):
        raise QueueError("not_archived", "l'épisode est hors de l'archive Podcasts")
    return os.path.realpath(episode.path)


def active_count():
    return (
        PodcastTranscript.select()
        .where(PodcastTranscript.status.in_(ACTIVE))
        .count()
    )


def auto_budget_used_ms(at=None):
    cutoff = (at or now()) - timedelta(hours=24)
    total = (
        PodcastTranscript.select(fn.SUM(PodcastTranscript.duration_ms))
        .where(
            (PodcastTranscript.user.is_null(True))
            & (PodcastTranscript.queued_at >= cutoff)
        )
        .scalar()
    )
    return int(total or 0)


def _budget_state(values, at=None):
    limit = int(values.get("daily_minutes") or 0) * 60_000
    used = auto_budget_used_ms(at)
    return {"used_ms": used, "limit_ms": limit, "remaining_ms": max(0, limit - used) if limit else None}


def _transcript_of(episode_id):
    return PodcastTranscript.get_or_none(
        PodcastTranscript.episode == episode_id
    )


def _check_limits(episode, path, values):
    duration = audio_duration_ms(episode)
    if not duration:
        raise QueueError(
            "no_duration", "la durée de l'épisode est inconnue ; impossible de la borner"
        )
    if duration > values["max_minutes"] * 60_000:
        raise QueueError(
            "too_long",
            f"l'épisode dépasse la durée maximale ({values['max_minutes']} min)",
        )
    try:
        size = os.path.getsize(path)
    except OSError as exc:
        raise QueueError("not_archived", "le fichier archivé est illisible") from exc
    if size > MAX_BYTES:
        raise QueueError("too_big", "l'épisode dépasse 1 Gio")
    return duration


def _no_local_runner(values):
    """True when this process will not run whisper itself.

    ``external`` has no engine here by design; a multi-worker deployment
    refuses the inline runner (see ``runner.inline_blocked``), so a queued job
    waits for a sidecar there too — saying "waiting" is what the client needs
    instead of a progress bar that never moves.
    """
    if values["runner"] == "external":
        return True
    from . import runner as _runner

    return _runner.workers() > 1


def enqueue(episode_id, user=None, force=False, language=None, app=None,
            source="server"):
    """Put one archived episode in the queue; never run the engine here."""
    values = settings.load(app)
    if not values["enabled"]:
        raise QueueError("disabled", "la transcription est désactivée")
    eid = episode_key(episode_id)
    episode = PodcastEpisode.get_or_none(PodcastEpisode.id == eid)
    if episode is None:
        raise QueueError("unknown_episode", "épisode inconnu")
    root = settings.archive_root(app)
    path = archive_file(episode, root)
    duration = _check_limits(episode, path, values)
    if values["runner"] == "inline":
        verdict = engine.quick_check(settings.paths(app))
        if not verdict["ok"]:
            raise QueueError(
                verdict["reason"], verdict["message"], detail=verdict["message"]
            )

    explicit = engine.normalize_language(language) if language else None
    transcript = _transcript_of(episode.id)
    if transcript is not None:
        if transcript.status in ACTIVE:
            return {
                "status": transcript.status,
                "episode_id": eid,
                "position": None,
                "waiting": _no_local_runner(values),
            }
        if transcript.status == "ready" and not force:
            raise QueueError("ready", "cet épisode est déjà transcrit")

    if transcript is None and active_count() >= QUEUE_MAX:
        raise QueueError(
            "queue_full",
            "la file de transcription est pleine",
            retry_in=60,
        )
    if (
        transcript is not None
        and transcript.status in ("error", "stopped", "ready")
        and active_count() >= QUEUE_MAX
    ):
        raise QueueError(
            "queue_full",
            "la file de transcription est pleine",
            retry_in=60,
        )

    when = now()
    source = source if source in SOURCES else "server"
    with db.atomic():
        if transcript is None:
            transcript = PodcastTranscript.create(
                episode=episode,
                status="queued",
                language=explicit,
                duration_ms=duration,
                done_ms=0,
                n_segments=0,
                user=user,
                queued_at=when,
                updated=when,
                source=source,
            )
        else:
            if transcript.status == "ready" and force:
                PodcastTranscriptSegment.delete().where(
                    PodcastTranscriptSegment.transcript == eid
                ).execute()
                transcript.done_ms = 0
                transcript.n_segments = 0
                transcript.engine = None
                transcript.model = None
                transcript.engine_version = None
                transcript.language = explicit
                transcript.source = source
                transcript.worker_name = None
                transcript.transcript_path = None
            elif transcript.status in ("error", "stopped"):
                if explicit is not None:
                    transcript.language = explicit
            transcript.duration_ms = duration
            transcript.status = "queued"
            transcript.queued_at = when
            transcript.started_at = None
            transcript.finished_at = None
            transcript.cancel_requested = False
            transcript.error_code = None
            transcript.error_message = None
            transcript.worker_name = None
            transcript.updated = when
            transcript.save()
    position = (
        PodcastTranscript.select()
        .where(PodcastTranscript.status == "queued")
        .count()
    )
    return {
        "status": "queued",
        "episode_id": eid,
        "position": position,
        "waiting": _no_local_runner(values),
    }


def auto_enqueue(episode, app=None):
    """The archive hook: one row for a new episode of a marked show.

    Never raises, never runs the engine, and creates nothing when the budget,
    the queue or the engine say no — a skipped episode stays ``none`` (not
    "waiting") and a later ``resume_auto`` can pick it up.
    """
    try:
        values = settings.load(app)
        if not values["enabled"]:
            return {"status": "skipped", "reason": "disabled", "episode_id": str(episode.id)}
        if not episode.channel.auto_transcribe:
            return {"status": "skipped", "reason": "show_off", "episode_id": str(episode.id)}
        existing = _transcript_of(episode.id)
        if existing is not None and existing.status in ACTIVE + ("ready",):
            return {
                "status": "skipped",
                "reason": "exists",
                "episode_id": str(episode.id),
                "current": existing.status,
            }
        root = settings.archive_root(app)
        if not root or not episode.path or not os.path.isfile(episode.path):
            return {"status": "skipped", "reason": "not_archived", "episode_id": str(episode.id)}
        if not sidecar.inside(episode.path, root):
            return {"status": "skipped", "reason": "not_archived", "episode_id": str(episode.id)}
        duration = audio_duration_ms(episode)
        if not duration:
            return {"status": "skipped", "reason": "no_duration", "episode_id": str(episode.id)}
        if duration > values["max_minutes"] * 60_000:
            return {"status": "skipped", "reason": "too_long", "episode_id": str(episode.id)}
        try:
            if os.path.getsize(episode.path) > MAX_BYTES:
                return {"status": "skipped", "reason": "too_big", "episode_id": str(episode.id)}
        except OSError:
            return {"status": "skipped", "reason": "not_archived", "episode_id": str(episode.id)}
        if active_count() >= QUEUE_MAX:
            return {"status": "skipped", "reason": "queue_full", "episode_id": str(episode.id)}
        if values["runner"] == "inline":
            verdict = engine.quick_check(settings.paths(app))
            if not verdict["ok"]:
                return {
                    "status": "skipped",
                    "reason": verdict["reason"],
                    "episode_id": str(episode.id),
                }
        limit = int(values["daily_minutes"] or 0) * 60_000
        if limit and auto_budget_used_ms() + duration > limit:
            return {"status": "skipped", "reason": "budget", "episode_id": str(episode.id)}
        out = enqueue(episode.id, user=None, app=app)
        try:
            from . import runner

            runner.wake()
        except Exception:
            logger.debug("could not wake the transcription runner", exc_info=True)
        return out
    except QueueError as exc:
        return {"status": "skipped", "reason": exc.code, "episode_id": str(episode.id)}
    except Exception as exc:
        logger.debug("transcription enqueue failed: %s", exc, exc_info=True)
        return {"status": "skipped", "reason": "error", "episode_id": str(episode.id)}


def resolve_language(transcript, episode, values):
    explicit = engine.normalize_language(transcript.language)
    if explicit and explicit != "auto":
        return explicit
    from_show = engine.normalize_language(episode.channel.transcribe_language)
    if from_show and from_show != "auto":
        return from_show
    global_value = engine.normalize_language(values.get("language"))
    return global_value or "auto"


def _lease_for(episode_id, worker, token):
    lease = PodcastTranscriptLease.get_or_none(
        PodcastTranscriptLease.episode == episode_id
    )
    if lease is None:
        raise LeaseError("lease_expired", "aucun lease pour cet épisode")
    worker_id = getattr(worker, "id", worker)
    if str(lease.worker_id) != str(worker_id):
        raise LeaseError("lease_expired", "ce worker ne détient pas ce lease")
    expected = hashlib.sha256(str(token or "").encode("utf-8")).hexdigest()
    if not hmac.compare_digest(lease.lease_token_hash, expected):
        raise LeaseError("lease_expired", "jeton de lease invalide")
    holder = lease.worker
    if not holder.enabled or holder.revoked is not None:
        raise LeaseError("lease_expired", "ce worker est révoqué")
    if lease.expires_at is None or lease.expires_at <= now():
        raise LeaseError("lease_expired", "ce lease a expiré")
    return lease


def _transcript_for(lease):
    try:
        return lease.episode
    except PodcastTranscript.DoesNotExist:
        return None


def _busy():
    from ..deezer import workload

    return (
        workload.streams_active() > 0
        or workload.foreground_busy()
        or workload.listening_for() < 15
        or workload.load_per_core() > 0.8
    )


def expire_leases(at=None):
    moment = at or now()
    expired = list(
        PodcastTranscriptLease.select()
        .where(PodcastTranscriptLease.expires_at <= moment)
        .order_by(PodcastTranscriptLease.expires_at)
    )
    changed = 0
    for lease in expired:
        transcript = _transcript_for(lease)
        if transcript is None:
            lease.delete_instance()
            continue
        if transcript.status not in ("leased", "queued"):
            # A finished or stopped job left its lease behind: no claim on that
            # row any more, and listing it under the worker's jobs would be a
            # lie. A `queued` row KEEPS its lease on purpose — it is the
            # attempt counter the third expiry turns into ``lease_lost``.
            lease.delete_instance()
            continue
        if transcript.status != "leased":
            continue
        lease.attempts = int(lease.attempts or 0) + 1
        lease.last_error = "lease_expired"
        eid = transcript.episode_id
        still_leased = (PodcastTranscript.episode == eid) & (
            PodcastTranscript.status == "leased"
        )
        if transcript.cancel_requested:
            moved = (
                PodcastTranscript.update(
                    status="stopped", finished_at=moment, updated=moment
                )
                .where(still_leased)
                .execute()
            )
            lease.delete_instance()
        elif lease.attempts >= LEASE_ATTEMPTS:
            moved = (
                PodcastTranscript.update(
                    status="error",
                    error_code="lease_lost",
                    error_message="lease expiré sans heartbeat",
                    finished_at=moment,
                    updated=moment,
                )
                .where(still_leased)
                .execute()
            )
            lease.delete_instance()
        else:
            moved = (
                PodcastTranscript.update(
                    status="queued", started_at=None, updated=moment
                )
                .where(still_leased)
                .execute()
            )
            if moved:
                lease.expires_at = moment
                lease.heartbeat_at = moment
                lease.save()
            else:
                lease.delete_instance()
        if moved:
            changed += 1
    return changed


def _live_server_lease():
    return (
        PodcastTranscriptLease.select()
        .join(TranscribeWorker)
        .where(
            (TranscribeWorker.kind == "server")
            & (PodcastTranscriptLease.expires_at > now())
        )
        .exists()
    )


def _claim_atomic():
    """The claim's transaction, taking SQLite's write lock up front.

    Both claimers read the queue before they write. Under SQLite's deferred
    ``BEGIN`` the second one then cannot upgrade its read lock while the first
    holds RESERVED, and the deadlock comes back as ``database is locked``
    immediately (the busy handler does not help a lock upgrade) — a 500 where
    the lost race must be a clean 204. ``BEGIN IMMEDIATE`` serialises the two
    claims instead; the other engines take their row locks normally.
    """
    if isinstance(db.obj, SqliteDatabase):
        return db.atomic("IMMEDIATE")
    return db.atomic()


def claim_next(worker, app=None):
    """Give one queued episode to an external worker, or None."""
    if isinstance(worker, TranscribeWorker):
        holder = worker
    else:
        holder = TranscribeWorker.get_or_none(TranscribeWorker.id == worker)
    if holder is None or not holder.enabled or holder.revoked is not None:
        raise LeaseError("lease_expired", "worker inconnu ou révoqué")
    when = now()
    alive = (
        TranscribeWorker.update(last_seen=when)
        .where(
            (TranscribeWorker.id == holder.id)
            & (TranscribeWorker.enabled == True)  # noqa: E712
            & TranscribeWorker.revoked.is_null()
        )
        .execute()
    )
    if not alive:
        raise LeaseError("lease_expired", "worker inconnu ou révoqué")
    expire_leases()
    if holder.kind == "server":
        if _live_server_lease():
            return None
        if PodcastTranscript.select().where(PodcastTranscript.status == "running").exists():
            return None
    for _attempt in range(5):
        # The move and its lease are ONE transaction: a crash in between used
        # to leave a `leased` row with no lease, a slot nobody could reclaim
        # (expire_leases walks the leases that exist). The worker's liveness
        # update above stays outside it, so a rolled-back claim still counts as
        # the worker being seen.
        with _claim_atomic():
            candidate = (
                PodcastTranscript.select()
                .where(PodcastTranscript.status == "queued")
                .order_by(
                    PodcastTranscript.user.is_null(),
                    PodcastTranscript.queued_at,
                    PodcastTranscript.episode,
                )
                .first()
            )
            if candidate is None:
                return None
            moved = (
                PodcastTranscript.update(
                    status="leased",
                    started_at=when,
                    updated=when,
                    cancel_requested=False,
                    source=holder.kind if holder.kind in SOURCES else "server",
                    worker_name=(holder.name or "")[:64] or None,
                )
                .where(
                    (PodcastTranscript.episode == candidate.episode_id)
                    & (PodcastTranscript.status == "queued")
                )
                .execute()
            )
            if not moved:
                continue
            token = secrets.token_urlsafe(24)
            digest = hashlib.sha256(token.encode("utf-8")).hexdigest()
            lease = PodcastTranscriptLease.get_or_none(
                PodcastTranscriptLease.episode == candidate.episode_id
            )
            attempts = int(lease.attempts or 0) if lease is not None else 0
            if lease is None:
                lease = PodcastTranscriptLease.create(
                    episode=candidate.episode_id,
                    worker=holder,
                    lease_token_hash=digest,
                    acquired_at=when,
                    expires_at=when + timedelta(seconds=LEASE_SECONDS),
                    heartbeat_at=when,
                    chunk_start_ms=candidate.done_ms,
                    attempts=attempts,
                )
            else:
                lease.worker = holder
                lease.lease_token_hash = digest
                lease.acquired_at = when
                lease.expires_at = when + timedelta(seconds=LEASE_SECONDS)
                lease.heartbeat_at = when
                lease.chunk_start_ms = candidate.done_ms
                lease.attempts = attempts
                lease.last_error = None
                lease.save()
        values = settings.load(app)
        return {
            "lease": token,
            "episode_id": str(candidate.episode_id),
            "expires_at": lease.expires_at,
            "start_ms": candidate.done_ms,
            "chunk_seconds": engine.CHUNK_SECONDS,
            "overlap_seconds": engine.OVERLAP_SECONDS,
            "language": resolve_language(candidate, candidate.episode, values),
            "duration_ms": candidate.duration_ms,
            "title": candidate.episode.title,
            "size": os.path.getsize(candidate.episode.path)
            if candidate.episode.path and os.path.isfile(candidate.episode.path)
            else -1,
        }
    return None


def claim_inline(app=None):
    """Take one queued episode for the in-process runner, or None."""
    expire_leases()
    if _live_server_lease():
        return None
    when = now()
    for _attempt in range(5):
        candidate = (
            PodcastTranscript.select()
            .where(PodcastTranscript.status == "queued")
            .order_by(
                PodcastTranscript.user.is_null(),
                PodcastTranscript.queued_at,
                PodcastTranscript.episode,
            )
            .first()
        )
        if candidate is None:
            return None
        moved = (
            PodcastTranscript.update(
                status="running",
                started_at=when,
                updated=when,
                cancel_requested=False,
                source="server",
                worker_name="inline",
            )
            .where(
                (PodcastTranscript.episode == candidate.episode_id)
                & (PodcastTranscript.status == "queued")
            )
            .execute()
        )
        if moved:
            PodcastTranscriptLease.delete().where(
                PodcastTranscriptLease.episode == candidate.episode_id
            ).execute()
            return PodcastTranscript.get(
                PodcastTranscript.episode == candidate.episode_id
            )
    return None


def heartbeat(episode_id, worker, token, chunk_pct=None):
    lease = _lease_for(episode_key(episode_id), worker, token)
    transcript = _transcript_for(lease)
    if transcript is None:
        raise LeaseError("lease_expired", "transcription absente")
    when = now()
    holder = lease.worker
    alive = (
        TranscribeWorker.update(last_seen=when)
        .where(
            (TranscribeWorker.id == holder.id)
            & (TranscribeWorker.enabled == True)  # noqa: E712
            & TranscribeWorker.revoked.is_null()
        )
        .execute()
    )
    if not alive:
        raise LeaseError("lease_expired", "worker inconnu ou révoqué")
    if lease.heartbeat_at is None or (
        when - lease.heartbeat_at
    ).total_seconds() >= HEARTBEAT_WRITE_SECONDS:
        lease.heartbeat_at = when
        lease.expires_at = when + timedelta(seconds=LEASE_SECONDS)
        lease.chunk_start_ms = transcript.done_ms
        lease.save()
    if chunk_pct is not None:
        try:
            note_progress(
                episode_id, transcript.done_ms, CHUNK_MS, float(chunk_pct)
            )
        except (TypeError, ValueError):
            pass
    return {
        "cancel": bool(transcript.cancel_requested),
        "pause": lease.worker.kind == "server" and _busy(),
        "expires_at": lease.expires_at,
        "done_ms": transcript.done_ms,
    }


def _segment_row(transcript_id, idx, raw, chunk_start, chunk_duration):
    if not isinstance(raw, dict):
        return None
    try:
        start = int(raw.get("start_ms"))
        end = int(raw.get("end_ms"))
    except (TypeError, ValueError):
        return None
    if start < chunk_start - OVERLAP_MS or start > chunk_start + chunk_duration:
        return None
    if end <= start or end > chunk_start + chunk_duration + OVERLAP_MS:
        return None
    text = clean_text(raw.get("text"))
    if not text:
        return None
    return {
        "transcript": transcript_id,
        "idx": idx,
        "start_ms": start,
        "end_ms": end,
        "text": text,
        "search_text": fold(text),
    }


def _insert_segments(rows, batch=200):
    for start in range(0, len(rows), batch):
        PodcastTranscriptSegment.insert_many(rows[start : start + batch]).execute()


def apply_chunk(episode_id, worker, token, start_ms, duration_ms, segments,
                language=None, engine_name=None, model=None, engine_version=None):
    """Store one chunk, idempotently, and advance ``done_ms`` server-side."""
    lease = _lease_for(episode_key(episode_id), worker, token)
    transcript = _transcript_for(lease)
    if transcript is None or transcript.status != "leased":
        raise LeaseError("lease_expired", "ce lease n'est plus actif")
    eid = transcript.episode_id
    try:
        start = int(start_ms)
        length = int(duration_ms)
    except (TypeError, ValueError) as exc:
        raise QueueError("chunk_out_of_order", "chunk invalide") from exc
    if start < transcript.done_ms:
        return {
            "applied": False,
            "done_ms": transcript.done_ms,
            "n_segments": transcript.n_segments,
        }
    if start > transcript.done_ms:
        raise QueueError("chunk_out_of_order", "ce chunk n'est pas le prochain")
    if not isinstance(segments, list):
        raise QueueError("chunk_out_of_order", "segments invalides")
    if len(segments) > engine.MAX_SEGMENTS_PER_CHUNK:
        raise QueueError("too_many_segments", "trop de segments dans ce chunk")
    length = max(1, min(length, CHUNK_MS))
    done = min(start + length, transcript.duration_ms or start + length)
    rows = []
    idx = int(transcript.n_segments or 0)
    for raw in segments:
        row = _segment_row(eid, idx, raw, start, length)
        if row is not None:
            idx += 1
            rows.append(row)
    when = now()
    updates = {
        "done_ms": done,
        "n_segments": idx,
        "updated": when,
        "language": engine.normalize_language(language) or transcript.language,
        "engine": (str(engine_name)[:16] if engine_name else transcript.engine),
        "model": (str(model)[:64] if model else transcript.model),
        "engine_version": (
            str(engine_version)[:32] if engine_version else transcript.engine_version
        ),
        "source": lease.worker.kind if lease.worker.kind in SOURCES else "server",
        "worker_name": (lease.worker.name or "")[:64] or None,
    }
    with db.atomic():
        moved = (
            PodcastTranscript.update(**updates)
            .where(
                (PodcastTranscript.episode == eid)
                & (PodcastTranscript.done_ms == start)
                & (PodcastTranscript.status == "leased")
            )
            .execute()
        )
        if not moved:
            current = PodcastTranscript.get(PodcastTranscript.episode == eid)
            return {
                "applied": False,
                "done_ms": current.done_ms,
                "n_segments": current.n_segments,
            }
        if rows:
            _insert_segments(rows)
        lease.chunk_start_ms = done
        lease.expires_at = when + timedelta(seconds=LEASE_SECONDS)
        lease.heartbeat_at = when
        lease.save()
    return {
        "applied": True,
        "done_ms": done,
        "n_segments": idx,
    }


def complete(episode_id, worker, token, language=None, engine_name=None,
             model=None, engine_version=None):
    """Mark a leased transcript ready; the caller flushes the sidecar off-request."""
    lease = _lease_for(episode_key(episode_id), worker, token)
    transcript = _transcript_for(lease)
    if transcript is None:
        raise LeaseError("lease_expired", "transcription absente")
    if transcript.done_ms < (transcript.duration_ms or 0) - 5000:
        raise QueueError(
            "incomplete",
            "le transcript n'est pas complet",
            detail={"done_ms": transcript.done_ms, "duration_ms": transcript.duration_ms},
        )
    when = now()
    transcript.status = "ready"
    transcript.finished_at = when
    transcript.updated = when
    transcript.error_code = None
    transcript.error_message = None
    transcript.cancel_requested = False
    transcript.language = engine.normalize_language(language) or transcript.language
    if engine_name:
        transcript.engine = str(engine_name)[:16]
    if model:
        transcript.model = str(model)[:64]
    if engine_version:
        transcript.engine_version = str(engine_version)[:32]
    transcript.source = lease.worker.kind if lease.worker.kind in SOURCES else "server"
    transcript.worker_name = (lease.worker.name or "")[:64] or None
    transcript.save()
    lease.delete_instance()
    return {
        "status": "ready",
        "done_ms": transcript.done_ms,
        "n_segments": transcript.n_segments,
    }


def fail(episode_id, worker, token, code, message=""):
    lease = _lease_for(episode_key(episode_id), worker, token)
    transcript = _transcript_for(lease)
    if transcript is None:
        raise LeaseError("lease_expired", "transcription absente")
    when = now()
    transcript.status = "error"
    transcript.error_code = (str(code or "exit")[:24]) or "exit"
    transcript.error_message = (str(message or "")[:300]) or None
    transcript.finished_at = when
    transcript.updated = when
    transcript.cancel_requested = False
    transcript.save()
    lease.delete_instance()
    return {"status": "error", "done_ms": transcript.done_ms}


def release(episode_id, worker, token):
    lease = _lease_for(episode_key(episode_id), worker, token)
    transcript = _transcript_for(lease)
    if transcript is None:
        raise LeaseError("lease_expired", "transcription absente")
    when = now()
    if transcript.cancel_requested:
        transcript.status = "stopped"
        transcript.finished_at = when
        status = "stopped"
    else:
        transcript.status = "queued"
        transcript.started_at = None
        status = "queued"
    transcript.updated = when
    transcript.save()
    lease.delete_instance()
    return {"status": status, "done_ms": transcript.done_ms}


def lease_audio(episode_id, worker, token, app=None):
    """The archived file of the leased episode, for the worker holding it.

    ``None`` means there is no lease at all (the caller answers 404); a lease
    held by somebody else, an expired lease or a revoked worker raises
    :class:`LeaseError` (the caller answers 403). The path never leaves this
    process except as bytes: the route streams the file.
    """
    eid = episode_key(episode_id)
    if PodcastTranscriptLease.get_or_none(
        PodcastTranscriptLease.episode == eid
    ) is None:
        return None
    lease = _lease_for(eid, worker, token)
    transcript = _transcript_for(lease)
    if transcript is None:
        return None
    return archive_file(transcript.episode, settings.archive_root(app))


def drop_worker_leases(worker):
    """Free what a revoked worker held: its jobs go back to the queue."""
    if isinstance(worker, TranscribeWorker):
        holder = worker
    else:
        holder = TranscribeWorker.get_or_none(TranscribeWorker.id == worker)
    if holder is None:
        return 0
    when = now()
    released = 0
    with db.atomic():
        for lease in list(
            PodcastTranscriptLease.select().where(
                PodcastTranscriptLease.worker == holder
            )
        ):
            transcript = _transcript_for(lease)
            if transcript is not None and transcript.status == "leased":
                updates = {"updated": when, "started_at": None, "status": "queued"}
                if transcript.cancel_requested:
                    updates = {"updated": when, "status": "stopped"}
                    updates["finished_at"] = when
                moved = (
                    PodcastTranscript.update(**updates)
                    .where(
                        (PodcastTranscript.episode == transcript.episode_id)
                        & (PodcastTranscript.status == "leased")
                    )
                    .execute()
                )
                if moved:
                    released += 1
            lease.delete_instance()
    return released


def mark_error(episode_id, code, message="", app=None):
    transcript = _transcript_of(episode_key(episode_id))
    if transcript is None:
        return None
    when = now()
    transcript.status = "error"
    transcript.error_code = (str(code or "exit")[:24]) or "exit"
    transcript.error_message = (str(message or "")[:300]) or None
    transcript.finished_at = when
    transcript.updated = when
    transcript.cancel_requested = False
    transcript.save()
    return transcript


def mark_stopped(episode_id):
    transcript = _transcript_of(episode_key(episode_id))
    if transcript is None:
        return None
    when = now()
    transcript.status = "stopped"
    transcript.finished_at = when
    transcript.updated = when
    transcript.cancel_requested = False
    transcript.save()
    return transcript


def mark_ready(episode_id, transcript_path=None, language=None, model=None,
               engine_version=None):
    transcript = _transcript_of(episode_key(episode_id))
    if transcript is None:
        return None
    when = now()
    transcript.status = "ready"
    transcript.finished_at = when
    transcript.updated = when
    transcript.error_code = None
    transcript.error_message = None
    transcript.cancel_requested = False
    if transcript_path:
        transcript.transcript_path = transcript_path
    if language:
        transcript.language = engine.normalize_language(language) or transcript.language
    if model:
        transcript.model = str(model)[:64]
    if engine_version:
        transcript.engine_version = str(engine_version)[:32]
    transcript.engine = engine.FORMAT_ENGINE
    remaining = (transcript.duration_ms or 0) - (transcript.done_ms or 0)
    if 0 < remaining <= 5000:
        transcript.done_ms = transcript.duration_ms
    transcript.save()
    return transcript


def append_inline(episode_id, start_ms, duration_ms, segments, language=None,
                  model=None, engine_version=None):
    """The inline runner's own chunk write: no lease, the same conditional UPDATE."""
    eid = episode_key(episode_id)
    transcript = _transcript_of(eid)
    if transcript is None or transcript.status != "running":
        raise QueueError("not_running", "cette transcription n'est plus en cours")
    start = int(start_ms)
    length = max(1, min(int(duration_ms), CHUNK_MS))
    if start < transcript.done_ms:
        return {
            "applied": False,
            "done_ms": transcript.done_ms,
            "n_segments": transcript.n_segments,
        }
    if start > transcript.done_ms:
        raise QueueError("chunk_out_of_order", "ce chunk n'est pas le prochain")
    done = min(start + length, transcript.duration_ms or start + length)
    rows = []
    idx = int(transcript.n_segments or 0)
    for raw in segments or ():
        row = _segment_row(eid, idx, raw, start, length)
        if row is not None:
            idx += 1
            rows.append(row)
    when = now()
    with db.atomic():
        moved = (
            PodcastTranscript.update(
                done_ms=done,
                n_segments=idx,
                updated=when,
                language=engine.normalize_language(language) or transcript.language,
                engine=engine.FORMAT_ENGINE,
                model=(str(model)[:64] if model else transcript.model),
                engine_version=(
                    str(engine_version)[:32]
                    if engine_version
                    else transcript.engine_version
                ),
                source="server",
                worker_name="inline",
            )
            .where(
                (PodcastTranscript.episode == eid)
                & (PodcastTranscript.done_ms == start)
                & (PodcastTranscript.status == "running")
            )
            .execute()
        )
        if not moved:
            current = PodcastTranscript.get(PodcastTranscript.episode == eid)
            return {
                "applied": False,
                "done_ms": current.done_ms,
                "n_segments": current.n_segments,
            }
        if rows:
            _insert_segments(rows)
    return {"applied": True, "done_ms": done, "n_segments": idx}


def set_outcome(episode_id, language=None, model=None, engine_version=None):
    transcript = _transcript_of(episode_key(episode_id))
    if transcript is None:
        return None
    if language:
        transcript.language = engine.normalize_language(language) or transcript.language
    if model:
        transcript.model = str(model)[:64]
    if engine_version:
        transcript.engine_version = str(engine_version)[:32]
    transcript.engine = engine.FORMAT_ENGINE
    transcript.updated = now()
    transcript.save()
    return transcript


def stop(episode_id, app=None):
    transcript = _transcript_of(episode_key(episode_id))
    if transcript is None or transcript.status not in ACTIVE:
        raise QueueError("not_running", "cet épisode n'est pas en cours de transcription")
    when = now()
    if transcript.status == "queued":
        transcript.status = "stopped"
        transcript.finished_at = when
        transcript.cancel_requested = False
        transcript.updated = when
        transcript.save()
        return {"status": "stopped", "episode_id": str(transcript.episode_id)}
    transcript.cancel_requested = True
    transcript.updated = when
    transcript.save()
    return {"status": "stopping", "episode_id": str(transcript.episode_id)}


def stop_all(app=None):
    when = now()
    queued = (
        PodcastTranscript.update(
            status="stopped", finished_at=when, cancel_requested=False, updated=when
        )
        .where(PodcastTranscript.status == "queued")
        .execute()
    )
    active = (
        PodcastTranscript.update(cancel_requested=True, updated=when)
        .where(PodcastTranscript.status.in_(("running", "leased")))
        .execute()
    )
    return {"stopped": queued, "stopping": active}


def resume_if_interrupted(app=None):
    """A restart leaves ``running`` rows behind; put them back in the queue.

    Also repairs the one state the old non-transactional claim could leave —
    ``leased`` with no lease row at all — since ``expire_leases`` can only walk
    leases that exist: such a row would occupy a slot for ever, with no button
    to free it. ``started_at`` older than a whole lease is the proof nobody is
    working on it any more; a live worker always has its lease row.
    """
    when = now()
    requeued = (
        PodcastTranscript.update(
            status="queued", started_at=None, updated=when
        )
        .where(PodcastTranscript.status == "running")
        .execute()
    )
    orphaned = (
        PodcastTranscript.update(
            status="queued", started_at=None, updated=when
        )
        .where(
            (PodcastTranscript.status == "leased")
            & (
                PodcastTranscript.started_at
                < when - timedelta(seconds=LEASE_SECONDS)
            )
            & PodcastTranscript.episode.not_in(
                PodcastTranscriptLease.select(PodcastTranscriptLease.episode)
            )
        )
        .execute()
    )
    expired = expire_leases()
    filled = resume_auto(app)
    return {
        "requeued": requeued + orphaned,
        "orphaned": orphaned,
        "expired": expired,
        "filled": filled,
    }


_progress_lock = threading.Lock()
_progress = {}


def note_progress(episode_id, chunk_start_ms, chunk_ms, chunk_pct):
    value = float(chunk_pct or 0.0)
    if value < 0 or value > 100:
        value = 0.0
    with _progress_lock:
        _progress[str(episode_id)] = {
            "chunk_start_ms": int(chunk_start_ms or 0),
            "chunk_ms": int(chunk_ms or 0),
            "chunk_pct": value,
        }


def clear_progress(episode_id):
    with _progress_lock:
        _progress.pop(str(episode_id), None)


def progress(episode_id):
    transcript = _transcript_of(episode_key(episode_id))
    if transcript is None:
        return None
    if transcript.status == "ready":
        return 1.0
    duration = int(transcript.duration_ms or 0)
    if duration <= 0:
        return 0.0
    with _progress_lock:
        live = _progress.get(str(episode_id))
    if live:
        done = live["chunk_start_ms"] + (
            live["chunk_ms"] * live["chunk_pct"] / 100.0
        )
    else:
        done = transcript.done_ms or 0
    return max(0.0, min(1.0, done / duration))


def episode_status(episode_id):
    try:
        eid = episode_key(episode_id)
    except QueueError:
        return {"episode_id": str(episode_id), "status": "none", "progress": 0.0}
    transcript = _transcript_of(eid)
    if transcript is None:
        episode = PodcastEpisode.get_or_none(PodcastEpisode.id == eid)
        return {
            "episode_id": eid,
            "status": "none",
            "title": episode.title if episode is not None else None,
            "progress": 0.0,
            "queued": False,
        }
    return {
        "episode_id": str(transcript.episode_id),
        "status": transcript.status,
        "title": transcript.episode.title,
        "language": transcript.language,
        "engine": transcript.engine,
        "model": transcript.model,
        "source": transcript.source,
        "worker": transcript.worker_name,
        "done_ms": transcript.done_ms,
        "duration_ms": transcript.duration_ms,
        "n_segments": transcript.n_segments,
        "error": transcript.error_code,
        "error_message": transcript.error_message,
        "transcript_path": transcript.transcript_path,
        "progress": progress(transcript.episode_id),
        "queued": transcript.status in ACTIVE,
    }


def queue_status(app=None):
    values = settings.load(app)
    counts = {status: 0 for status in STATUSES}
    for status, number in (
        PodcastTranscript.select(
            PodcastTranscript.status, fn.COUNT(PodcastTranscript.episode)
        )
        .group_by(PodcastTranscript.status)
        .tuples()
    ):
        counts[status] = number
    active = []
    for transcript in (
        PodcastTranscript.select()
        .where(PodcastTranscript.status.in_(ACTIVE))
        .order_by(PodcastTranscript.queued_at)
        .limit(QUEUE_MAX)
    ):
        active.append(
            {
                "episode_id": str(transcript.episode_id),
                "status": transcript.status,
                "title": transcript.episode.title,
                "done_ms": transcript.done_ms,
                "duration_ms": transcript.duration_ms,
                "progress": progress(transcript.episode_id),
                "error": transcript.error_code,
            }
        )
    budget = _budget_state(values)
    available, reason = settings.available(app)
    if available and values["runner"] == "inline":
        from . import runner as _runner

        if _runner.workers() > 1:
            # The inline runner refuses a multi-worker deployment: the queue
            # would sit there with nothing to move it.
            available, reason = False, "multi_worker"
    return {
        "enabled": values["enabled"],
        "runner": values["runner"],
        "available": available,
        "reason": reason,
        "counts": counts,
        "active": active,
        "budget": budget,
    }


def _candidate_query(channel=None, limit=None):
    blocked = PodcastTranscript.select(PodcastTranscript.episode).where(
        PodcastTranscript.status.not_in(("error", "stopped"))
    )
    query = (
        PodcastEpisode.select(PodcastEpisode, PodcastChannel)
        .join(PodcastChannel)
        .where(
            (PodcastChannel.auto_transcribe == True)  # noqa: E712 - a SQL expression
            & (PodcastEpisode.path.is_null(False))
            & (PodcastEpisode.status == "completed")
            & PodcastEpisode.id.not_in(blocked)
        )
        .order_by(
            fn.COALESCE(PodcastEpisode.publish_date, PodcastEpisode.created).desc(),
            PodcastEpisode.created.desc(),
        )
    )
    if channel is not None:
        query = query.where(PodcastChannel.id == getattr(channel, "id", channel))
    if limit:
        query = query.limit(limit)
    return query


def _blocked_ids():
    return {
        str(episode_id)
        for (episode_id,) in PodcastTranscript.select(
            PodcastTranscript.episode
        )
        .where(PodcastTranscript.status.not_in(("error", "stopped")))
        .tuples()
    }


def candidates(channel=None, limit=None, app=None):
    blocked = _blocked_ids()
    out = []
    for episode in _candidate_query(channel=channel, limit=limit):
        if str(episode.id) in blocked:
            continue
        out.append(episode)
    return out


def estimate(channel=None, app=None):
    values = settings.load(app)
    episodes = candidates(channel=channel, app=app)
    seconds = sum(int(episode.duration or 0) for episode in episodes)
    return {
        "episodes": len(episodes),
        "audio_seconds": seconds,
        "audio_hours": round(seconds / 3600.0, 1),
        "cpu_hours": round(seconds / 3600.0 / NOMINAL_XRT, 1),
        "daily_minutes": values["daily_minutes"],
        "max_minutes": values["max_minutes"],
    }


def queue_backlog(channel=None, backlog="all", user=None, app=None, limit=None):
    """Queue what marking a show means: none, the 10 last, or everything.

    ``all`` is the resumable BULK job: it fills the free queue slots under the
    budget, and ``resume_auto`` refills as jobs finish. A caller never waits on
    it — the rows are written and the runner is woken.
    """
    mode = str(backlog or settings.BACKLOG_DEFAULT).strip().lower()
    if mode not in settings.BACKLOG_MODES:
        raise QueueError("invalid_backlog", "backlog doit être all, recent ou none")
    summary = {"queued": 0, "skipped": {}, "considered": 0, "backlog": mode}
    if mode == "none":
        return summary
    wanted = 10 if mode == "recent" else (limit or QUEUE_MAX)
    episodes = candidates(channel=channel, limit=wanted * 3, app=app)
    for episode in episodes:
        if summary["queued"] >= wanted:
            break
        if active_count() >= QUEUE_MAX:
            summary["skipped"]["queue_full"] = summary["skipped"].get("queue_full", 0) + 1
            break
        summary["considered"] += 1
        try:
            if user is None:
                outcome = auto_enqueue(episode, app=app)
                if outcome.get("status") == "queued":
                    summary["queued"] += 1
                else:
                    reason = outcome.get("reason") or "skipped"
                    summary["skipped"][reason] = summary["skipped"].get(reason, 0) + 1
                    if reason in ("budget", "queue_full"):
                        break
            else:
                enqueue(episode.id, user=user, app=app)
                summary["queued"] += 1
        except QueueError as exc:
            summary["skipped"][exc.code] = summary["skipped"].get(exc.code, 0) + 1
    if summary["queued"]:
        try:
            from . import runner

            runner.wake()
        except Exception:
            logger.debug("could not wake the transcription runner", exc_info=True)
    return summary


def _stored_backlog(channel):
    key = "tr.bl." + str(channel.id)[:24]
    row = Meta.get_or_none(Meta.key == key)
    value = row.value if row is not None else None
    return value if value in ("all", "recent", "none") else "all"


def resume_auto(app=None):
    """Fill the free queue slots from marked shows, each with its stored backlog."""
    values = settings.load(app)
    if not values["enabled"]:
        return 0
    if values["runner"] == "inline" and not engine.quick_check(settings.paths(app))["ok"]:
        return 0
    channels = list(
        PodcastChannel.select().where(PodcastChannel.auto_transcribe == True)  # noqa: E712
    )
    if not channels:
        return 0
    free = QUEUE_MAX - active_count()
    if free <= 0:
        return 0
    queued = 0
    for channel in channels:
        if queued >= free:
            break
        summary = queue_backlog(
            channel=channel,
            backlog=_stored_backlog(channel),
            user=None,
            app=app,
            limit=free - queued,
        )
        queued += summary["queued"]
    return queued


def sidecar_payload(transcript):
    segments = [
        {"start_ms": row.start_ms, "end_ms": row.end_ms, "text": row.text}
        for row in transcript.segments.order_by(PodcastTranscriptSegment.idx)
    ]
    return sidecar.build(
        episode_id=transcript.episode_id,
        language=transcript.language,
        engine=transcript.engine or engine.FORMAT_ENGINE,
        model=transcript.model,
        engine_version=transcript.engine_version,
        duration_ms=transcript.duration_ms,
        segments=segments,
        created=(transcript.queued_at or transcript.started_at or now()).isoformat(),
        updated=now().isoformat(),
    )


def flush_sidecar(episode_id, app=None):
    """Write the sidecar from the database; never from a request thread.

    Only ``transcript_path`` and ``updated`` are written by a targeted UPDATE:
    a background flush that finishes after a completion or a failure must not
    put the row back to the state it read before the disk write.
    """
    transcript = _transcript_of(episode_key(episode_id))
    if transcript is None:
        return None
    root = settings.archive_root(app)
    episode = transcript.episode
    if not root or not episode.path or not os.path.isfile(episode.path):
        return None
    try:
        payload = sidecar_payload(transcript)
        path = sidecar.write(episode.path, payload, root)
    except (OSError, sidecar.SidecarError) as exc:
        logger.info("Could not write the transcript sidecar: %s", exc)
        return None
    PodcastTranscript.update(
        transcript_path=path, updated=now()
    ).where(PodcastTranscript.episode == transcript.episode_id).execute()
    return path


def remove(episode_id, app=None):
    """Delete the derived rows and sidecars; never the audio."""
    transcript = _transcript_of(episode_key(episode_id))
    if transcript is None:
        return False
    if transcript.status in ACTIVE:
        raise QueueError("running", "arrêtez la transcription avant de la supprimer")
    episode = transcript.episode
    root = settings.archive_root(app)
    if root and episode.path:
        try:
            sidecar.remove(episode.path, root)
        except (OSError, sidecar.SidecarError):
            pass
    transcript.delete_instance()
    return True


delete = remove


def drain(app=None, limit=None):
    """Run queued jobs synchronously on this thread (tests and the CLI's tests)."""
    from . import runner

    return runner.drain(app, limit=limit)


def rebuild(app=None, limit=None):
    """Recreate the index from the sidecars on the data disk."""
    root = settings.archive_root(app)
    if not root or not os.path.isdir(root):
        raise QueueError(
            "no_archive",
            "l'archive de podcasts est absente (archive_dir/Podcasts)",
        )
    stats = {
        "scanned": 0,
        "rebuilt": 0,
        "skipped": 0,
        "failed": 0,
        "failure_reasons": {},
    }
    for directory, _dirs, names in os.walk(root, followlinks=False):
        for name in sorted(names):
            if not name.endswith(".transcript.json"):
                continue
            stats["scanned"] += 1
            path = os.path.join(directory, name)
            try:
                payload = sidecar.load_file(path, root)
            except sidecar.SidecarError as exc:
                stats["failed"] += 1
                reason = str(exc)
                stats["failure_reasons"][reason] = (
                    stats["failure_reasons"].get(reason, 0) + 1
                )
                continue
            try:
                rebuilt = _rebuild_one(payload, path, root)
            except Exception as exc:
                stats["failed"] += 1
                reason = f"{type(exc).__name__}: {exc}"[:120]
                stats["failure_reasons"][reason] = (
                    stats["failure_reasons"].get(reason, 0) + 1
                )
                continue
            if rebuilt:
                stats["rebuilt"] += 1
                if limit and stats["rebuilt"] >= limit:
                    return stats
            else:
                stats["skipped"] += 1
    return stats


def _rebuild_one(payload, path, root):
    try:
        eid = episode_key(payload.get("episode_id"))
    except QueueError:
        return False
    episode = PodcastEpisode.get_or_none(PodcastEpisode.id == eid)
    if episode is None or not episode.path:
        return False
    expected = os.path.splitext(os.path.realpath(episode.path))[0] + ".transcript.json"
    if os.path.realpath(path) != expected:
        return False
    transcript = _transcript_of(eid)
    if transcript is not None and transcript.status in ACTIVE:
        return False
    segments = payload["segments"]
    if (
        transcript is not None
        and transcript.status == "ready"
        and int(transcript.n_segments or 0) == len(segments)
        and transcript.transcript_path == os.path.realpath(path)
    ):
        return False
    when = now()
    duration = int(payload.get("duration_ms") or 0) or int(episode.duration or 0) * 1000
    with db.atomic():
        if transcript is None:
            transcript = PodcastTranscript.create(
                episode=episode,
                status="ready",
                language=engine.normalize_language(payload.get("language")),
                engine=(str(payload.get("engine"))[:16] if payload.get("engine") else engine.FORMAT_ENGINE),
                model=(str(payload.get("model"))[:64] if payload.get("model") else None),
                engine_version=(
                    str(payload.get("engine_version"))[:32]
                    if payload.get("engine_version")
                    else None
                ),
                duration_ms=duration,
                done_ms=duration,
                n_segments=0,
                source="server",
                transcript_path=os.path.realpath(path),
                queued_at=None,
                started_at=when,
                finished_at=when,
                updated=when,
            )
        else:
            PodcastTranscriptSegment.delete().where(
                PodcastTranscriptSegment.transcript == eid
            ).execute()
            transcript.status = "ready"
            transcript.language = engine.normalize_language(payload.get("language"))
            transcript.engine = (
                str(payload.get("engine"))[:16]
                if payload.get("engine")
                else engine.FORMAT_ENGINE
            )
            transcript.model = (
                str(payload.get("model"))[:64] if payload.get("model") else None
            )
            transcript.engine_version = (
                str(payload.get("engine_version"))[:32]
                if payload.get("engine_version")
                else None
            )
            transcript.duration_ms = duration
            transcript.done_ms = duration
            transcript.n_segments = 0
            transcript.error_code = None
            transcript.error_message = None
            transcript.cancel_requested = False
            transcript.finished_at = when
            transcript.updated = when
            transcript.transcript_path = os.path.realpath(path)
            transcript.save()
        rows = [
            {
                "transcript": eid,
                "idx": idx,
                "start_ms": segment["start_ms"],
                "end_ms": segment["end_ms"],
                "text": segment["text"],
                "search_text": fold(segment["text"]),
            }
            for idx, segment in enumerate(segments)
        ]
        _insert_segments(rows)
        transcript.n_segments = len(rows)
        transcript.save()
    return True
