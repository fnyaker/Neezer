# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""The transcription worker protocol: enrol a device, lease a job, deliver text.

- ``GET|POST /api/transcribe/workers`` and ``DELETE /api/transcribe/workers/<id>``
  (admin): list devices without their credential, create one and show its token
  **once**, revoke one. The token is stored encrypted with the server secret
  (``managers.user.encrypt_password``) and is never read back by a route.
- ``POST /api/transcribe/worker/claim`` and the per-episode
  ``heartbeat``/``chunk``/``complete``/``fail``/``release`` routes, plus
  ``GET /api/transcribe/worker/audio/<eid>``: a bearer device token picks the
  worker, and the lease token returned by ``claim`` proves that this worker owns
  this episode (``X-NS-Lease``, or a ``lease`` field in the JSON body). A
  revoked or disabled worker is refused before any queue work is read.

The container and an enrolled client speak this protocol and nothing else: they
receive audio as bytes over ``audio/<eid>`` (Range supported), never a path, and
they never see the database, the archive or another episode. Sidecars are
written by a background thread after a chunk or a completion: a request never
waits on the data disk. No provider is called here.
"""

from __future__ import annotations

import hmac
import logging
import re
import secrets
import threading
import uuid

from flask import Blueprint, current_app, jsonify, request, send_file
from werkzeug.exceptions import HTTPException, RequestEntityTooLarge

from ..db import (
    PodcastTranscript,
    PodcastTranscriptLease,
    TranscribeWorker,
    db,
    now,
)
from ..managers.user import decrypt_password, encrypt_password
from ..ratelimit import WindowLimiter
from ..transcribe import engine, job, settings
from . import admin_required, login_required, same_origin_guard
from .remote import NEVER, POLICY

logger = logging.getLogger(__name__)

MAX_BODY = 64 * 1024
MAX_LARGE_BODY = 1024 * 1024
MAX_WORKERS = 32
NAME_MAX = 64
KINDS = ("server", "client")

_CONTROL = re.compile(r"[\x00-\x1f\x7f]")
_SPACES = re.compile(r"\s+")
ADMIN_ENDPOINTS = frozenset(
    {"transcribe_workers", "transcribe_worker_create", "transcribe_worker_revoke"}
)
LARGE_ENDPOINTS = frozenset({"transcribe_worker_chunk", "transcribe_worker_complete"})

WORKER_POLICY = {
    "transcribe_workers": "admin",
    "transcribe_worker_create": "admin",
    "transcribe_worker_revoke": "admin",
    "transcribe_worker_claim": NEVER,
    "transcribe_worker_heartbeat": NEVER,
    "transcribe_worker_chunk": NEVER,
    "transcribe_worker_complete": NEVER,
    "transcribe_worker_fail": NEVER,
    "transcribe_worker_release": NEVER,
    "transcribe_worker_audio": NEVER,
}


class _WorkerBlueprint(Blueprint):
    def register(self, app, options):
        super().register(app, options)
        POLICY.update(WORKER_POLICY)


transcribe_worker_api = _WorkerBlueprint("transcribe_worker_api", __name__)

create_limiter = WindowLimiter(30, 3600)
revoke_limiter = WindowLimiter(30, 3600)
claim_limiter = WindowLimiter(12, 60)
heartbeat_limiter = WindowLimiter(30, 60)
chunk_limiter = WindowLimiter(12, 60)
complete_limiter = WindowLimiter(12, 60)
fail_limiter = WindowLimiter(12, 60)
release_limiter = WindowLimiter(12, 60)
audio_limiter = WindowLimiter(12, 60)

LIMITERS = (
    create_limiter,
    revoke_limiter,
    claim_limiter,
    heartbeat_limiter,
    chunk_limiter,
    complete_limiter,
    fail_limiter,
    release_limiter,
    audio_limiter,
)

_ENDPOINT_LIMITS = {
    "transcribe_worker_claim": claim_limiter,
    "transcribe_worker_heartbeat": heartbeat_limiter,
    "transcribe_worker_chunk": chunk_limiter,
    "transcribe_worker_complete": complete_limiter,
    "transcribe_worker_fail": fail_limiter,
    "transcribe_worker_release": release_limiter,
    "transcribe_worker_audio": audio_limiter,
}


class _Rejected(Exception):
    def __init__(self, message, code="invalid", status=400, extra=None):
        super().__init__(message)
        self.message = message
        self.code = code
        self.status = status
        self.extra = extra or {}


@transcribe_worker_api.before_request
def _prepare():
    endpoint = (request.endpoint or "").rsplit(".", 1)[-1]
    request.max_content_length = (
        MAX_LARGE_BODY if endpoint in LARGE_ENDPOINTS else MAX_BODY
    )
    if endpoint in ADMIN_ENDPOINTS:
        return same_origin_guard()
    return None


@transcribe_worker_api.after_request
def _no_store(response):
    response.headers["Cache-Control"] = "no-store"
    return response


@transcribe_worker_api.errorhandler(_Rejected)
def _reject(exc):
    payload = {"error": exc.message, "code": exc.code}
    payload.update(exc.extra)
    response = jsonify(payload)
    response.status_code = exc.status
    if exc.status == 401:
        response.headers["WWW-Authenticate"] = "Bearer"
    if exc.status == 429:
        response.headers["Retry-After"] = str(int(exc.extra.get("retry_in") or 1))
    return response


@transcribe_worker_api.errorhandler(RequestEntityTooLarge)
def _too_large(_exc):
    return jsonify({"error": "corps de requête trop gros", "code": "too_large"}), 413


@transcribe_worker_api.errorhandler(Exception)
def _json_error(exc):
    if isinstance(exc, HTTPException):
        return jsonify({"error": exc.name}), exc.code
    logger.error("Unhandled error on %s %s", request.method, request.path, exc_info=True)
    return jsonify({"error": "server error"}), 500


def _app():
    return current_app._get_current_object()


def _uuid(value):
    try:
        return uuid.UUID(str(value))
    except (ValueError, TypeError, AttributeError):
        return None


def _iso(value):
    return value.isoformat() if value is not None else None


def _text(value, limit):
    if not isinstance(value, str):
        return None
    text = _SPACES.sub(" ", _CONTROL.sub(" ", value)).strip()
    return text[:limit].strip() or None


def _clean_message(value, limit=240):
    return _text(value, limit)


def _json_body(allow_empty=False):
    data = request.get_json(silent=True)
    if data is None:
        data = {}
    if not isinstance(data, dict):
        raise _Rejected("corps JSON invalide")
    if not data and not allow_empty:
        raise _Rejected("aucun réglage")
    return data


def _number(data, name):
    if name not in data:
        raise _Rejected(f"{name} est requis", code=name)
    value = data[name]
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise _Rejected(f"{name} doit être un nombre", code=name)
    result = int(value)
    if result < 0 or result > 2_147_483_647:
        raise _Rejected(f"{name} est hors bornes", code=name)
    return result


def _rate(limiter, key):
    allowed, retry_in = limiter.allow(str(key))
    if not allowed:
        raise _Rejected(
            "trop de demandes",
            code="rate_limited",
            status=429,
            extra={"retry_in": retry_in},
        )


def _worker_rate(worker):
    endpoint = (request.endpoint or "").rsplit(".", 1)[-1]
    limiter = _ENDPOINT_LIMITS.get(endpoint)
    if limiter is not None:
        _rate(limiter, worker.id)


def _presented_token():
    header = request.headers.get("Authorization") or ""
    scheme, _, value = header.partition(" ")
    if scheme.lower() != "bearer":
        return None
    token = value.strip()
    if not token or len(token) > 200 or any(char.isspace() for char in token):
        return None
    return token


def _split_token(token):
    if not isinstance(token, str) or not token or len(token) > 200:
        return None
    prefix, separator, secret = token.partition(".")
    if not separator or not secret:
        return None
    ident = _uuid(prefix)
    return (str(ident), secret) if ident is not None else None


def _authenticate():
    token = _presented_token()
    if token is None:
        raise _Rejected("jeton d'appareil requis", code="unauthorized", status=401)
    parts = _split_token(token)
    if parts is None:
        raise _Rejected("jeton d'appareil invalide", code="unauthorized", status=401)
    worker = TranscribeWorker.get_or_none(TranscribeWorker.id == parts[0])
    if worker is None:
        raise _Rejected("jeton d'appareil invalide", code="unauthorized", status=401)
    try:
        expected = decrypt_password(worker.token).encode("utf-8")
    except Exception:
        expected = b""
    if not expected or not hmac.compare_digest(expected, token.encode("utf-8")):
        raise _Rejected("jeton d'appareil invalide", code="unauthorized", status=401)
    if not worker.enabled or worker.revoked is not None:
        raise _Rejected("cet appareil est révoqué", code="worker_revoked", status=403)
    return worker


def _lease_token(data=None):
    token = request.headers.get("X-NS-Lease") or ""
    if not token and isinstance(data, dict):
        candidate = data.get("lease")
        if isinstance(candidate, str):
            token = candidate
    token = token.strip()
    if not token or len(token) > 200 or any(char.isspace() for char in token):
        raise _Rejected(
            "jeton de lease requis", code="lease_required", status=403
        )
    return token


def _episode_id(value):
    ident = _uuid(value)
    if ident is None:
        raise _Rejected("épisode inconnu", code="not_found", status=404)
    return str(ident)


def _raise_queue_error(exc):
    if exc.code in ("invalid_id", "unknown_episode"):
        raise _Rejected("épisode inconnu", code="not_found", status=404)
    if exc.code in ("not_archived", "no_archive"):
        raise _Rejected("épisode non archivé", code="not_archived", status=404)
    raise _Rejected(
        exc.message,
        code=exc.code,
        status=409,
        extra=dict(exc.detail or {}),
    )


def _worker_brief(worker):
    return {
        "id": str(worker.id),
        "name": worker.name,
        "kind": worker.kind,
        "created": _iso(worker.created),
    }


def _lease_brief(lease):
    if lease is None:
        return None
    return {
        "episode_id": str(lease.episode_id),
        "expires_at": _iso(lease.expires_at),
        "heartbeat_at": _iso(lease.heartbeat_at),
        "attempts": int(lease.attempts or 0),
    }


def _worker_row(worker, lease):
    return {
        **_worker_brief(worker),
        "enabled": bool(worker.enabled),
        "last_seen": _iso(worker.last_seen),
        "revoked": _iso(worker.revoked),
        "lease": _lease_brief(lease),
    }


_flush_lock = threading.Lock()


def _flush_sidecar(episode_id, app):
    try:
        db.connect(reuse_if_open=True)
    except Exception:
        logger.debug("sidecar thread could not open the database", exc_info=True)
    try:
        with _flush_lock:
            job.flush_sidecar(episode_id, app)
    except Exception:
        logger.info("Could not write the transcript sidecar", exc_info=True)
    finally:
        try:
            db.close()
        except Exception:
            pass


def _flush_async(episode_id, app):
    thread = threading.Thread(
        target=_flush_sidecar,
        args=(episode_id, app),
        name="transcribe-sidecar",
        daemon=True,
    )
    try:
        thread.start()
    except RuntimeError:
        logger.info("Could not start the transcript sidecar thread", exc_info=True)
        return None
    return thread


@transcribe_worker_api.route("/transcribe/workers")
@login_required
@admin_required
def transcribe_workers():
    workers = list(TranscribeWorker.select().order_by(TranscribeWorker.created.desc()))
    leases = {
        str(lease.worker_id): lease
        for lease in PodcastTranscriptLease.select()
    }
    return jsonify(
        {"workers": [_worker_row(row, leases.get(str(row.id))) for row in workers]}
    )


@transcribe_worker_api.route("/transcribe/workers", methods=["POST"])
@login_required
@admin_required
def transcribe_worker_create():
    _rate(create_limiter, request.webuser.id)
    data = _json_body()
    raw_name = data.get("name")
    if not isinstance(raw_name, str):
        raise _Rejected("name est requis", code="name")
    if any(ord(char) < 32 or ord(char) == 127 for char in raw_name):
        raise _Rejected("name contient un caractère interdit", code="name")
    name = _SPACES.sub(" ", raw_name).strip()
    if not name or len(name) > NAME_MAX:
        raise _Rejected(
            f"name doit faire entre 1 et {NAME_MAX} caractères", code="name"
        )
    kind = data.get("kind")
    if kind not in KINDS:
        raise _Rejected("kind doit être server ou client", code="kind")
    if (
        TranscribeWorker.select()
        .where(TranscribeWorker.revoked.is_null(True))
        .count()
        >= MAX_WORKERS
    ):
        raise _Rejected(
            f"{MAX_WORKERS} appareils au maximum", code="too_many_workers", status=409
        )
    with db.atomic():
        worker = TranscribeWorker.create(name=name, kind=kind, token="pending")
        token = f"{worker.id}.{secrets.token_urlsafe(32)}"
        worker.token = encrypt_password(token)
        worker.save()
    response = jsonify({"worker": _worker_brief(worker), "token": token})
    response.status_code = 201
    return response


@transcribe_worker_api.route("/transcribe/workers/<worker_id>", methods=["DELETE"])
@login_required
@admin_required
def transcribe_worker_revoke(worker_id):
    _rate(revoke_limiter, request.webuser.id)
    ident = _uuid(worker_id)
    if ident is None:
        raise _Rejected("appareil inconnu", code="not_found", status=404)
    worker = TranscribeWorker.get_or_none(TranscribeWorker.id == str(ident))
    if worker is None:
        raise _Rejected("appareil inconnu", code="not_found", status=404)
    revoked = worker.revoked is None
    released = 0
    if revoked:
        worker.revoked = now()
        worker.enabled = False
        worker.save()
        released = job.drop_worker_leases(worker)
    return jsonify(
        {"id": str(worker.id), "revoked": revoked, "released": released}
    )


@transcribe_worker_api.route("/transcribe/worker/claim", methods=["POST"])
def transcribe_worker_claim():
    worker = _authenticate()
    _worker_rate(worker)
    try:
        result = job.claim_next(worker, app=_app())
    except job.QueueError as exc:
        _raise_queue_error(exc)
    if result is None:
        return ("", 204)
    values = settings.load(_app())
    return jsonify(
        {
            "episode_id": result["episode_id"],
            "lease": result["lease"],
            "audio_url": "/api/transcribe/worker/audio/" + result["episode_id"],
            "duration_ms": int(result["duration_ms"] or 0),
            "language": result["language"],
            "chunk_ms": int(result["chunk_seconds"]) * 1000,
            "overlap_ms": int(result["overlap_seconds"]) * 1000,
            "max_minutes": int(values["max_minutes"]),
            "start_ms": int(result["start_ms"] or 0),
            "expires_at": _iso(result["expires_at"]),
        }
    )


@transcribe_worker_api.route(
    "/transcribe/worker/<eid>/heartbeat", methods=["POST"]
)
def transcribe_worker_heartbeat(eid):
    worker = _authenticate()
    _worker_rate(worker)
    data = _json_body(allow_empty=True)
    chunk_pct = data.get("chunk_pct")
    if chunk_pct is not None:
        if isinstance(chunk_pct, bool) or not isinstance(chunk_pct, (int, float)):
            raise _Rejected("chunk_pct doit être un nombre", code="chunk_pct")
        chunk_pct = max(0.0, min(100.0, float(chunk_pct)))
    token = _lease_token(data)
    try:
        beat = job.heartbeat(
            _episode_id(eid), worker, token, chunk_pct=chunk_pct
        )
    except job.QueueError as exc:
        _raise_queue_error(exc)
    return jsonify(
        {
            "cancel": bool(beat["cancel"]),
            "pause": bool(beat["pause"]),
            "done_ms": int(beat["done_ms"] or 0),
            "expires_at": _iso(beat["expires_at"]),
        }
    )


@transcribe_worker_api.route("/transcribe/worker/<eid>/chunk", methods=["POST"])
def transcribe_worker_chunk(eid):
    worker = _authenticate()
    _worker_rate(worker)
    episode_id = _episode_id(eid)
    data = _json_body()
    start = _number(data, "start_ms")
    length = _number(data, "duration_ms")
    segments = data.get("segments")
    if not isinstance(segments, list):
        raise _Rejected("segments doit être une liste", code="segments")
    if len(segments) > engine.MAX_SEGMENTS_PER_CHUNK:
        raise _Rejected(
            "trop de segments dans ce chunk",
            code="too_many_segments",
            status=409,
        )
    token = _lease_token(data)
    try:
        result = job.apply_chunk(
            episode_id,
            worker,
            token,
            start,
            length,
            segments,
            language=engine.normalize_language(data.get("language")),
            engine_name=_text(data.get("engine"), 16),
            model=_text(data.get("model"), 64),
            engine_version=_text(data.get("engine_version"), 32),
        )
    except job.QueueError as exc:
        _raise_queue_error(exc)
    if result.get("applied"):
        _flush_async(episode_id, _app())
    return jsonify(result)


def _apply_tail(episode_id, worker, token, data, segments):
    transcript = PodcastTranscript.get_or_none(
        PodcastTranscript.episode == episode_id
    )
    if transcript is None:
        return
    remaining = int(transcript.duration_ms or 0) - int(transcript.done_ms or 0)
    if remaining <= 0:
        return
    try:
        job.apply_chunk(
            episode_id,
            worker,
            token,
            int(transcript.done_ms or 0),
            min(job.CHUNK_MS, remaining),
            segments,
            language=engine.normalize_language(data.get("language")),
            engine_name=_text(data.get("engine"), 16),
            model=_text(data.get("model"), 64),
            engine_version=_text(data.get("engine_version"), 32),
        )
    except job.QueueError as exc:
        if exc.code == "chunk_out_of_order":
            return
        raise


@transcribe_worker_api.route(
    "/transcribe/worker/<eid>/complete", methods=["POST"]
)
def transcribe_worker_complete(eid):
    worker = _authenticate()
    _worker_rate(worker)
    episode_id = _episode_id(eid)
    data = _json_body(allow_empty=True)
    segments = data.get("segments")
    if segments is not None:
        if not isinstance(segments, list):
            raise _Rejected("segments doit être une liste", code="segments")
        if len(segments) > engine.MAX_SEGMENTS_PER_CHUNK:
            raise _Rejected(
                "trop de segments dans ce chunk",
                code="too_many_segments",
                status=409,
            )
    token = _lease_token(data)
    try:
        if segments:
            _apply_tail(episode_id, worker, token, data, segments)
        result = job.complete(
            episode_id,
            worker,
            token,
            language=engine.normalize_language(data.get("language")),
            engine_name=_text(data.get("engine"), 16),
            model=_text(data.get("model"), 64),
            engine_version=_text(data.get("engine_version"), 32),
        )
    except job.QueueError as exc:
        _raise_queue_error(exc)
    _flush_async(episode_id, _app())
    return jsonify(result)


@transcribe_worker_api.route("/transcribe/worker/<eid>/fail", methods=["POST"])
def transcribe_worker_fail(eid):
    worker = _authenticate()
    _worker_rate(worker)
    episode_id = _episode_id(eid)
    data = _json_body()
    code = _text(data.get("code"), 24)
    if not code:
        raise _Rejected("code est requis", code="code")
    message = _clean_message(data.get("message"))
    token = _lease_token(data)
    try:
        result = job.fail(episode_id, worker, token, code, message or "")
    except job.QueueError as exc:
        _raise_queue_error(exc)
    return jsonify(result)


@transcribe_worker_api.route("/transcribe/worker/<eid>/release", methods=["POST"])
def transcribe_worker_release(eid):
    worker = _authenticate()
    _worker_rate(worker)
    episode_id = _episode_id(eid)
    data = _json_body(allow_empty=True)
    token = _lease_token(data)
    try:
        result = job.release(episode_id, worker, token)
    except job.QueueError as exc:
        _raise_queue_error(exc)
    return jsonify(result)


@transcribe_worker_api.route("/transcribe/worker/audio/<eid>")
def transcribe_worker_audio(eid):
    worker = _authenticate()
    _worker_rate(worker)
    episode_id = _episode_id(eid)
    token = _lease_token()
    try:
        path = job.lease_audio(episode_id, worker, token, app=_app())
    except job.LeaseError:
        raise _Rejected(
            "aucun lease actif pour cet épisode",
            code="lease_expired",
            status=403,
        ) from None
    except job.QueueError:
        raise _Rejected(
            "épisode non archivé", code="not_archived", status=404
        ) from None
    if path is None:
        raise _Rejected("épisode inconnu", code="not_found", status=404)
    try:
        response = send_file(path, conditional=True)
    except OSError:
        raise _Rejected(
            "épisode non archivé", code="not_archived", status=404
        ) from None
    response.headers["Cache-Control"] = "no-store"
    return response
