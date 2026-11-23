# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Podcast transcription: the queue's API, the settings and the text.

- ``GET /api/transcribe/status`` (any account): whether the feature can serve
  work, the queue counters, the public model name and, for an admin alone, the
  last failure's code and a path-scrubbed cause.
- ``GET|PUT /api/transcribe/settings`` (admin): the effective values and the
  five an admin may change (``enabled``, ``language``, ``max_minutes``,
  ``daily_minutes``, ``search_segments``). ``PUT`` writes ``Meta`` rows
  (``tr.*``) and starts no work; a path is never read from a body.
- ``POST /api/podcast/<pid>/transcribe`` (admin): the show's automatic switch,
  its language, its backlog mode, and the bounded backlog run it queues.
- ``POST /api/podcast/episode/<eid>/transcribe`` (admin) starts or resumes one
  episode, ``/stop`` asks the runner to stop, ``DELETE .../transcript`` removes
  the derived text and sidecars and never the audio.
- ``GET /api/podcast/episode/<eid>/transcript`` (any account) is one page of
  timed lines, ``at`` opens the page around a position, ``q`` filters the
  episode.
- ``GET /api/podcast/<pid>/transcript/search`` and ``/api/transcripts/search``
  answer bounded excerpts with a timecode; the library route says how much of
  the library it actually read.

Only the queue, the settings and read-only text live here: no provider is
called, no engine process is started in a request, and no payload ever names a
sidecar or engine path.
"""

from __future__ import annotations

import logging
import os
import uuid

from flask import Blueprint, current_app, jsonify, request
from peewee import fn

from ..db import (
    Meta,
    PodcastChannel,
    PodcastEpisode,
    PodcastTranscript,
    PodcastTranscriptSegment,
    db,
)
from ..ratelimit import WindowLimiter
from ..transcribe import engine, job, settings
from . import _image, admin_required, login_required, same_origin_guard
from .remote import NEVER, POLICY

logger = logging.getLogger(__name__)

MAX_BODY = 8 * 1024
PAGE_DEFAULT = 100
PAGE_MAX = 200
SEARCH_DEFAULT = 20
SEARCH_MAX = 50
SHOW_EPISODES = 100
LIBRARY_EPISODES = 2000
Q_MIN = 2
Q_MAX = 200
Q_WORDS = 6
EXCERPT_CHARS = 240
LANGUAGES = ["auto", "fr", "en", "es", "de", "it", "pt"]

SETTING_KEYS = ("enabled", "language", "max_minutes", "daily_minutes", "search_segments")
PATH_SETTINGS = {
    "bin",
    "model",
    "vad_model",
    "vad",
    "ffmpeg",
    "cpuset",
    "path",
    "paths",
    "archive",
    "cache",
    "root",
}
SERVER_SETTINGS = {"runner"}
META_PREFIX = "tr."
BACKLOG_PREFIX = "tr.bl."

mark_limiter = WindowLimiter(1, 600)
start_limiter = WindowLimiter(3, 600)
start_hour_limiter = WindowLimiter(10, 3600)
search_limiter = WindowLimiter(30, 60)
panel_search_limiter = WindowLimiter(60, 60)
read_limiter = WindowLimiter(120, 60)

TRANSCRIBE_POLICY = {
    "transcribe_status": "read",
    "transcribe_settings": NEVER,
    "auto_transcribe": NEVER,
    "start_episode_transcript": NEVER,
    "stop_episode_transcript": NEVER,
    "delete_episode_transcript": NEVER,
    "episode_transcript": "read",
    "show_transcript_search": "read",
    "podcast_search": "read",
}


class _TranscribeBlueprint(Blueprint):
    def register(self, app, options):
        super().register(app, options)
        POLICY.update(TRANSCRIBE_POLICY)


transcribe_api = _TranscribeBlueprint("transcribe_api", __name__)


class _Rejected(Exception):
    def __init__(self, message, code="invalid", status=400, extra=None):
        super().__init__(message)
        self.message = message
        self.code = code
        self.status = status
        self.extra = extra or {}


@transcribe_api.before_request
def reject_cross_site():
    request.max_content_length = MAX_BODY
    return same_origin_guard()


@transcribe_api.errorhandler(_Rejected)
def _reject(exc):
    payload = {"error": exc.message, "code": exc.code}
    payload.update(exc.extra)
    resp = jsonify(payload)
    resp.status_code = exc.status
    if exc.status == 429:
        resp.headers["Retry-After"] = str(int(exc.extra.get("retry_in") or 1))
    return resp


@transcribe_api.errorhandler(Exception)
def _json_error(exc):
    from werkzeug.exceptions import HTTPException

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


def _episode_or_none(eid):
    ident = _uuid(eid)
    if ident is None:
        return None
    return PodcastEpisode.get_or_none(PodcastEpisode.id == str(ident))


def _channel_or_none(pid):
    ident = _uuid(pid)
    if ident is not None:
        row = PodcastChannel.get_or_none(PodcastChannel.id == str(ident))
        if row is not None:
            return row
    text = str(pid or "")
    if text.isdigit():
        return PodcastChannel.get_or_none(PodcastChannel.deezer_id == text)
    return None


def _json_body(allow_empty=False):
    data = request.get_json(silent=True)
    if not isinstance(data, dict):
        raise _Rejected("corps JSON invalide")
    if not data and not allow_empty:
        raise _Rejected("aucun réglage")
    return data


def _int_arg(name, default, low, high):
    raw = request.args.get(name)
    if raw in (None, ""):
        return default
    try:
        value = int(str(raw))
    except (TypeError, ValueError):
        raise _Rejected(f"{name} doit être un entier", code=name) from None
    return max(low, min(high, value))


def _float_arg(name, default):
    raw = request.args.get(name)
    if raw in (None, ""):
        return default
    try:
        value = float(str(raw))
    except (TypeError, ValueError):
        raise _Rejected(f"{name} doit être un nombre", code=name) from None
    if value < 0:
        raise _Rejected(f"{name} doit être positif", code=name)
    return value


def _words(raw):
    text = str(raw or "").strip()
    if not text:
        return None
    if len(text) < Q_MIN:
        raise _Rejected(
            f"{Q_MIN} caractères minimum pour chercher dans les transcriptions",
            code="q",
        )
    if len(text) > Q_MAX:
        raise _Rejected("requête trop longue", code="q")
    folded = job.fold(text).split()
    if not folded or any(len(word) < Q_MIN for word in folded):
        raise _Rejected(
            f"{Q_MIN} caractères minimum pour chercher dans les transcriptions",
            code="q",
        )
    if len(folded) > Q_WORDS:
        raise _Rejected(f"{Q_WORDS} mots maximum", code="q")
    return folded, text


def _language(data, key="language"):
    raw = data.get(key)
    if raw is None or raw == "auto":
        return None
    if not isinstance(raw, str):
        raise _Rejected("language doit être un code de langue", code="language")
    code = engine.normalize_language(raw)
    if not code:
        raise _Rejected(f"langue inconnue : {raw}", code="language")
    return None if code == "auto" else code


def _rate(limiter):
    allowed, retry_in = limiter.allow(str(request.webuser.id))
    if not allowed:
        raise _Rejected(
            "trop de demandes",
            code="rate_limited",
            status=429,
            extra={"retry_in": retry_in},
        )


def _seconds(milliseconds):
    return round(int(milliseconds or 0) / 1000.0, 3)


def _scrub(text):
    value = str(text or "")[:300]
    if not value:
        return None
    try:
        paths = settings.paths(_app())
        extra = settings.cache_dir(_app())
    except Exception:
        paths, extra = {}, None
    candidates = [
        paths.get(name)
        for name in ("bin", "model", "ffmpeg", "archive", "cache", "root")
    ]
    for path in candidates + [extra]:
        if isinstance(path, str) and len(path) > 3:
            value = value.replace(path, os.path.basename(path.rstrip(os.sep)) or "…")
    return value


def _error_payload(code, message, admin):
    if not code:
        return None
    out = {"code": code}
    if admin and message:
        scrubbed = _scrub(message)
        if scrubbed:
            out["message"] = scrubbed
    return out


def _progress(status, duration_ms, done_ms, episode_id):
    if status == "ready":
        return 1.0
    if status in ("running", "leased"):
        live = job.progress(episode_id)
        if live is not None:
            return float(live)
    duration = int(duration_ms or 0)
    if duration <= 0:
        return 0.0
    return max(0.0, min(1.0, int(done_ms or 0) / duration))


def _readiness(values, paths, report=None):
    """(available, reason, destination, report): will a queued episode ever run?

    One place answers it for the status payload, the settings payload and the
    activation route, so the three can never disagree. The local engine is not
    the only place transcription can happen: an enrolled device (a PC on the
    LAN, the sidecar container) claims the queue over the worker protocol, and
    its own engine, model and ffmpeg are the ones that matter. **Enrolment is
    what counts, never liveness** — a PC that is off at this second claims the
    queued episode when it comes back, and a failure to reach it is not a
    verdict about it.
    """
    report = report if report is not None else job.workers_report()
    if values["runner"] == "external":
        destination = "external"
    elif engine.quick_check(paths)["ok"]:
        destination = "server"
    elif report["enrolled"]:
        destination = "workers"
    else:
        destination = None
    if not values["enabled"]:
        return False, "disabled", destination, report
    if destination is None:
        return False, "no_worker", None, report
    return True, None, destination, report


def _workers_payload(report):
    return {"enrolled": int(report["enrolled"]), "online": int(report["online"])}


#: What a refused activation says, per gate. The SPA, the desktop and the phone
#: print their own sentence from the `reason` code; this message is what an API
#: client and a log read.
READINESS_MESSAGES = {
    "disabled": "la transcription est désactivée",
    "no_worker": (
        "aucun ordinateur n'est enrôlé pour transcrire, et le serveur n'a pas "
        "de moteur"
    ),
}


def _queue_counts():
    counts = {status: 0 for status in job.STATUSES}
    query = (
        PodcastTranscript.select(
            PodcastTranscript.status, fn.COUNT(PodcastTranscript.episode)
        )
        .group_by(PodcastTranscript.status)
        .tuples()
    )
    for status, number in query:
        if status in counts:
            counts[status] = int(number or 0)
    return counts


def _queue_payload(values=None):
    values = values or settings.load(_app())
    limit = int(values["daily_minutes"] or 0) * 60_000
    return {
        **_queue_counts(),
        "budget_used_ms": int(job.auto_budget_used_ms()),
        "budget_limit_ms": limit,
    }


def _last_error():
    row = (
        PodcastTranscript.select(
            PodcastTranscript.error_code, PodcastTranscript.error_message
        )
        .where(PodcastTranscript.status == "error")
        .order_by(PodcastTranscript.updated.desc())
        .first()
    )
    if row is None or not row.error_code:
        return None
    return {"code": row.error_code, "message": _scrub(row.error_message)}


def _status_payload():
    app = _app()
    values = settings.load(app)
    paths = settings.paths(app)
    available, reason, destination, report = _readiness(values, paths)
    payload = {
        "enabled": bool(values["enabled"]),
        "available": bool(available),
        "reason": reason,
        "runner": values["runner"],
        "destination": destination,
        "workers": _workers_payload(report),
        "language": values["language"],
        "max_minutes": values["max_minutes"],
        "daily_minutes": values["daily_minutes"],
        "search_segments": values["search_segments"],
        "model": engine.model_name(paths["model"]),
        "queue": _queue_payload(values),
    }
    if request.webuser.admin:
        payload["last_error"] = _last_error()
    return payload


def _settings_payload(fresh=False):
    app = _app()
    values = settings.load(app)
    paths = settings.paths(app)
    report = job.workers_report()
    available, reason, destination, _report = _readiness(values, paths, report)
    verdict = engine.probe(paths) if fresh else engine.quick_check(paths)
    model_bytes = int(verdict.get("model_bytes") or 0)
    engine_ok = (
        bool(paths.get("bin"))
        and os.path.isfile(paths["bin"])
        and os.access(paths["bin"], os.X_OK)
    )
    model_ok = bool(paths.get("model")) and engine.valid_model(paths["model"])
    return {
        "enabled": bool(values["enabled"]),
        "language": values["language"],
        "languages": list(LANGUAGES),
        "max_minutes": values["max_minutes"],
        "daily_minutes": values["daily_minutes"],
        "search_segments": values["search_segments"],
        "runner": values["runner"],
        "available": bool(available),
        "reason": reason,
        "destination": destination,
        "workers": _workers_payload(report),
        # The machines themselves are for the admin card alone: the status
        # payload (any account) carries the counts and never a hostname.
        "machines": list(report["machines"]),
        "engine": {
            "name": engine.FORMAT_ENGINE,
            "version": verdict.get("version"),
            "ok": engine_ok,
        },
        "model": {
            "name": engine.model_name(paths["model"]),
            "size_mb": round(model_bytes / (1024 * 1024.0)),
            "ok": model_ok,
        },
        "threads": settings.threads(),
        "queue": _queue_payload(values),
    }


def _meta_set(key, value):
    row = Meta.get_or_none(Meta.key == key)
    if row is None:
        Meta.create(key=key, value=value)
    else:
        row.value = value
        row.save()


def _store(updates):
    with db.atomic():
        for key, value in updates.items():
            _meta_set(META_PREFIX + key, value)


def _backlog_key(channel):
    return BACKLOG_PREFIX + str(channel.id)[:24]


def _show_backlog(channel):
    row = Meta.get_or_none(Meta.key == _backlog_key(channel))
    value = row.value if row is not None else None
    return value if value in settings.BACKLOG_MODES else settings.BACKLOG_DEFAULT


def _queue_error(exc):
    if exc.code in ("unknown_episode", "invalid_id"):
        return _Rejected(exc.message, code=exc.code, status=404)
    if exc.code == "queue_full":
        return _Rejected(
            exc.message,
            code=exc.code,
            status=503,
            extra={"retry_in": exc.retry_in or 60},
        )
    return _Rejected(exc.message, code=exc.code, status=409)


def _excerpt(text, words):
    value = text or ""
    folded = job.fold(value)
    positions = []
    if len(folded) == len(value):
        for word in words:
            found = folded.find(word)
            if found >= 0:
                positions.append((found, len(word)))
    if not positions:
        return value[:EXCERPT_CHARS]
    position = min(positions)[0]
    start = max(0, position - EXCERPT_CHARS // 2)
    end = min(len(value), start + EXCERPT_CHARS)
    start = max(0, end - EXCERPT_CHARS)
    out = value[start:end]
    if start > 0:
        out = "…" + out
    if end < len(value):
        out = out + "…"
    return out[:EXCERPT_CHARS]


def _score(text, words):
    folded = job.fold(text or "")
    return min(99, sum(folded.count(word) for word in words))


def _episode_card(row):
    (
        eid,
        _count,
        title,
        publish_date,
        created,
        image_md5,
        channel_id,
        channel_title,
        cover_md5,
        gone,
    ) = row
    cover = (
        "/api/cover/" + str(channel_id)
        if gone is not None
        else _image("talk", image_md5 or cover_md5)
    )
    published = 0
    if publish_date is not None:
        published = int(publish_date.timestamp())
    elif created is not None:
        published = int(created.timestamp())
    return {
        "id": str(eid),
        "title": title or "",
        "cover": cover,
        "published": published,
        "channel_id": str(channel_id),
        "channel_title": channel_title or "",
    }


def _search_clause(column, words):
    clause = None
    for word in words:
        part = column.contains(word)
        clause = part if clause is None else clause & part
    return clause


def _segment_hits(ids, words, limit):
    query = (
        PodcastTranscriptSegment.select(
            PodcastTranscriptSegment.idx,
            PodcastTranscriptSegment.transcript,
            PodcastTranscriptSegment.start_ms,
            PodcastTranscriptSegment.end_ms,
            PodcastTranscriptSegment.text,
        )
        .join(
            PodcastTranscript,
            on=(PodcastTranscript.episode == PodcastTranscriptSegment.transcript),
        )
        .join(PodcastEpisode, on=(PodcastEpisode.id == PodcastTranscript.episode))
        .where(
            (PodcastTranscriptSegment.transcript << ids)
            & _search_clause(PodcastTranscriptSegment.search_text, words)
        )
        .order_by(
            fn.COALESCE(PodcastEpisode.publish_date, PodcastEpisode.created).desc(),
            PodcastTranscriptSegment.start_ms,
            PodcastTranscriptSegment.idx,
        )
        .limit(limit + 1)
        .tuples()
    )
    rows = list(query)
    more = len(rows) > limit
    hits = []
    for idx, eid, start_ms, end_ms, text in rows[:limit]:
        hits.append(
            {
                "idx": idx,
                "start": _seconds(start_ms),
                "end": _seconds(end_ms),
                "text": _excerpt(text, words),
                "score": _score(text, words),
                "_episode_id": str(eid),
            }
        )
    return hits, more


def _attach_episodes(hits, cards):
    for hit in hits:
        eid = hit.pop("_episode_id")
        card = cards.get(eid)
        hit["episode_id"] = eid
        hit["episode"] = (
            {
                "id": card["id"],
                "title": card["title"],
                "cover": card["cover"],
                "published": card["published"],
            }
            if card
            else None
        )
        hit["channel_id"] = card["channel_id"] if card else None
        hit["channel_title"] = card["channel_title"] if card else None
    return hits


@transcribe_api.route("/transcribe/status")
@login_required
def transcribe_status():
    return jsonify(_status_payload())


@transcribe_api.route("/transcribe/settings", methods=["GET", "PUT"])
@login_required
@admin_required
def transcribe_settings():
    if request.method == "GET":
        return jsonify(_settings_payload(fresh=request.args.get("probe") == "1"))
    data = _json_body()
    for key in data:
        if key in PATH_SETTINGS:
            raise _Rejected(
                "les chemins du moteur se règlent dans le fichier de configuration",
                code="paths_are_server_side",
            )
        if key in SERVER_SETTINGS:
            raise _Rejected(
                "ce réglage se règle dans le fichier de configuration",
                code="server_side_setting",
            )
        if key not in SETTING_KEYS:
            raise _Rejected(f"réglage inconnu : {key}", code="unknown_setting")
    updates = {}
    if "enabled" in data:
        if not isinstance(data["enabled"], bool):
            raise _Rejected("enabled doit être true ou false", code="enabled")
        updates["enabled"] = "1" if data["enabled"] else "0"
    if "language" in data:
        raw = data["language"]
        if not isinstance(raw, str):
            raise _Rejected("language doit être un code de langue", code="language")
        code = engine.normalize_language(raw)
        if not code:
            raise _Rejected(f"langue inconnue : {raw}", code="language")
        updates["language"] = code
    for key, low, high in (
        ("max_minutes", 5, settings.MAX_LIMIT_MINUTES),
        ("daily_minutes", 0, settings.MAX_DAILY_MINUTES),
        ("search_segments", 0, settings.MAX_SEARCH_SEGMENTS),
    ):
        if key not in data:
            continue
        value = data[key]
        if isinstance(value, bool) or not isinstance(value, int):
            raise _Rejected(f"{key} doit être un entier", code=key)
        if not low <= value <= high:
            raise _Rejected(
                f"{key} doit être compris entre {low} et {high}", code=key
            )
        updates[key] = str(value)
    if data.get("enabled") is True:
        # Activation is allowed on capacity, not on the local engine: a server
        # with no whisper.cpp but an enrolled PC (even one that is off right
        # now) will serve every queued episode -- the PC claims it.
        values = dict(settings.load(_app()), enabled=True)
        available, reason, _destination, _report = _readiness(
            values, settings.paths(_app())
        )
        if not available:
            raise _Rejected(
                READINESS_MESSAGES.get(
                    reason, "le moteur de transcription est indisponible"
                ),
                code="unavailable",
                status=409,
                extra={"reason": reason},
            )
    if not updates:
        raise _Rejected("aucun réglage")
    _store(updates)
    settings.invalidate()
    return jsonify(_settings_payload())


@transcribe_api.route("/podcast/<pid>/transcribe", methods=["POST"])
@login_required
@admin_required
def auto_transcribe(pid):
    channel = _channel_or_none(pid)
    if channel is None:
        raise _Rejected("émission inconnue", code="not_found", status=404)
    data = _json_body()
    if "auto" in data and not isinstance(data["auto"], bool):
        raise _Rejected("auto doit être true ou false", code="auto")
    has_language = "language" in data
    language = _language(data) if has_language else None
    if "backlog" in data and data["backlog"] not in settings.BACKLOG_MODES:
        raise _Rejected("backlog doit être all, recent ou none", code="backlog")
    if "auto" not in data and not has_language and "backlog" not in data:
        raise _Rejected("aucun réglage")
    auto = data.get("auto")
    if auto is True:
        _rate(mark_limiter)
    if has_language:
        channel.transcribe_language = language
    if "auto" in data:
        channel.auto_transcribe = auto
    channel.save(
        only=(PodcastChannel.auto_transcribe, PodcastChannel.transcribe_language)
    )
    if "backlog" in data:
        _meta_set(_backlog_key(channel), data["backlog"])
    mode = _show_backlog(channel)
    from . import _channel

    if auto is True:
        estimate = job.estimate(channel=channel, app=_app())
        summary = job.queue_backlog(
            channel=channel, backlog=mode, user=None, app=_app()
        )
        return (
            jsonify(
                {
                    **_channel(channel),
                    "backlog": mode,
                    "queued": summary["queued"],
                    "skipped": summary["skipped"],
                    "estimate": estimate,
                }
            ),
            202,
        )
    return jsonify({**_channel(channel), "backlog": mode, "queued": 0})


@transcribe_api.route("/podcast/episode/<eid>/transcribe", methods=["POST"])
@login_required
@admin_required
def start_episode_transcript(eid):
    episode = _episode_or_none(eid)
    if episode is None:
        raise _Rejected("épisode inconnu", code="not_found", status=404)
    data = _json_body(allow_empty=True)
    language = _language(data) if "language" in data else None
    force = data.get("force", False)
    if not isinstance(force, bool):
        raise _Rejected("force doit être true ou false", code="force")
    current = PodcastTranscript.get_or_none(PodcastTranscript.episode == episode.id)
    if current is not None and current.status in job.ACTIVE:
        raise _Rejected(
            "cet épisode est déjà en cours de transcription",
            code="already_running",
            status=409,
            extra={"status": current.status},
        )
    if current is not None and current.status == "ready" and not force:
        raise _Rejected("cet épisode est déjà transcrit", code="ready", status=409)
    _rate(start_limiter)
    _rate(start_hour_limiter)
    try:
        result = job.enqueue(
            episode.id,
            user=request.webuser,
            force=force,
            language=language,
            app=_app(),
        )
    except job.QueueError as exc:
        raise _queue_error(exc) from None
    from ..transcribe import runner

    runner.wake()
    return (
        jsonify(
            {
                "state": "queued",
                "episode_id": result["episode_id"],
                "status": result["status"],
                "position": result["position"],
                "waiting": result["waiting"],
            }
        ),
        202,
    )


@transcribe_api.route("/podcast/episode/<eid>/transcribe/stop", methods=["POST"])
@login_required
@admin_required
def stop_episode_transcript(eid):
    episode = _episode_or_none(eid)
    if episode is None:
        raise _Rejected("épisode inconnu", code="not_found", status=404)
    try:
        result = job.stop(episode.id, app=_app())
    except job.QueueError as exc:
        raise _queue_error(exc) from None
    return jsonify(result)


@transcribe_api.route("/podcast/episode/<eid>/transcript", methods=["DELETE"])
@login_required
@admin_required
def delete_episode_transcript(eid):
    episode = _episode_or_none(eid)
    if episode is None:
        raise _Rejected("épisode inconnu", code="not_found", status=404)
    try:
        job.remove(episode.id, app=_app())
    except job.QueueError as exc:
        raise _queue_error(exc) from None
    return ("", 204)


def _transcript_row(eid):
    return (
        PodcastTranscript.select(
            PodcastTranscript.status,
            PodcastTranscript.language,
            PodcastTranscript.engine,
            PodcastTranscript.model,
            PodcastTranscript.source,
            PodcastTranscript.worker_name,
            PodcastTranscript.duration_ms,
            PodcastTranscript.done_ms,
            PodcastTranscript.n_segments,
            PodcastTranscript.error_code,
            PodcastTranscript.error_message,
            PodcastEpisode.path,
            PodcastEpisode.duration,
        )
        .join(PodcastEpisode, on=(PodcastEpisode.id == PodcastTranscript.episode))
        .where(PodcastTranscript.episode == eid)
        .tuples()
        .first()
    )


@transcribe_api.route("/podcast/episode/<eid>/transcript")
@login_required
def episode_transcript(eid):
    ident = _uuid(eid)
    if ident is None:
        raise _Rejected("épisode inconnu", code="not_found", status=404)
    eid = str(ident)
    limit = _int_arg("limit", PAGE_DEFAULT, 1, PAGE_MAX)
    offset = _int_arg("offset", 0, 0, 1_000_000_000)
    at = _float_arg("at", None)
    found = _words(request.args.get("q"))
    words = found[0] if found else None
    if words:
        _rate(panel_search_limiter)
    else:
        _rate(read_limiter)

    row = _transcript_row(eid)
    if row is None:
        episode = _episode_or_none(eid)
        if episode is None:
            raise _Rejected("épisode inconnu", code="not_found", status=404)
        return jsonify(
            {
                "episode_id": eid,
                "status": "none",
                "archived": bool(episode.path and os.path.isfile(episode.path)),
                "language": None,
                "engine": None,
                "model": None,
                "source": None,
                "worker": None,
                "duration_ms": int(episode.duration or 0) * 1000,
                "done_ms": 0,
                "progress": 0.0,
                "total": 0,
                "offset": 0,
                "limit": limit,
                "query": None,
                "segments": [],
                "next": None,
            }
        )
    (
        status,
        language,
        engine_name,
        model,
        source,
        worker,
        duration_ms,
        done_ms,
        n_segments,
        error_code,
        error_message,
        path,
        episode_duration,
    ) = row
    archived = bool(path and os.path.isfile(path))
    base = PodcastTranscriptSegment.select(
        PodcastTranscriptSegment.idx,
        PodcastTranscriptSegment.start_ms,
        PodcastTranscriptSegment.end_ms,
        PodcastTranscriptSegment.text,
    ).where(PodcastTranscriptSegment.transcript == eid)
    total = int(n_segments or 0)
    if words:
        matches = base.where(
            _search_clause(PodcastTranscriptSegment.search_text, words)
        )
        total = matches.count()
        rows = list(
            matches.order_by(
                PodcastTranscriptSegment.start_ms, PodcastTranscriptSegment.idx
            )
            .offset(offset)
            .limit(limit)
            .tuples()
        )
    else:
        if at is not None and total:
            index = (
                PodcastTranscriptSegment.select(PodcastTranscriptSegment.idx)
                .where(
                    (PodcastTranscriptSegment.transcript == eid)
                    & (PodcastTranscriptSegment.start_ms <= int(at * 1000))
                )
                .order_by(
                    PodcastTranscriptSegment.start_ms.desc(),
                    PodcastTranscriptSegment.idx.desc(),
                )
                .scalar()
            )
            offset = (int(index) // limit) * limit if index is not None else 0
        rows = list(
            base.order_by(PodcastTranscriptSegment.idx)
            .offset(offset)
            .limit(limit)
            .tuples()
        )
    segments = [
        {
            "start": _seconds(start_ms),
            "end": _seconds(end_ms),
            "text": _excerpt(text, words) if words else text,
        }
        for _idx, start_ms, end_ms, text in rows
    ]
    consumed = offset + len(rows)
    return jsonify(
        {
            "episode_id": eid,
            "status": status,
            "archived": archived,
            "language": language,
            "engine": engine_name,
            "model": model,
            "source": source,
            "worker": worker,
            "duration_ms": int(duration_ms or (episode_duration or 0) * 1000),
            "done_ms": int(done_ms or 0),
            "progress": _progress(status, duration_ms, done_ms, eid),
            "total": total,
            "offset": offset,
            "limit": limit,
            "query": found[1] if found else None,
            "segments": segments,
            "next": consumed if consumed < total else None,
            "error": _error_payload(error_code, error_message, request.webuser.admin),
        }
    )


@transcribe_api.route("/podcast/<pid>/transcript/search")
@login_required
def show_transcript_search(pid):
    _rate(search_limiter)
    channel = _channel_or_none(pid)
    if channel is None:
        raise _Rejected("émission inconnue", code="not_found", status=404)
    limit = _int_arg("limit", SEARCH_DEFAULT, 1, SEARCH_MAX)
    found = _words(request.args.get("q"))
    if not found:
        return jsonify(
            {
                "q": "",
                "hits": [],
                "total": 0,
                "more": False,
                "scope": {"episodes": 0, "truncated": False},
            }
        )
    words, query = found
    rows = list(
        PodcastTranscript.select(
            PodcastTranscript.episode,
            PodcastTranscript.n_segments,
            PodcastEpisode.title,
            PodcastEpisode.publish_date,
            PodcastEpisode.created,
            PodcastEpisode.image_md5,
            PodcastChannel.id,
            PodcastChannel.title,
            PodcastChannel.cover_art_md5,
            PodcastChannel.gone,
        )
        .join(PodcastEpisode, on=(PodcastEpisode.id == PodcastTranscript.episode))
        .join(PodcastChannel, on=(PodcastChannel.id == PodcastEpisode.channel))
        .where(
            (PodcastTranscript.status == "ready")
            & (PodcastChannel.id == channel.id)
        )
        .order_by(
            fn.COALESCE(PodcastEpisode.publish_date, PodcastEpisode.created).desc(),
            PodcastTranscript.updated.desc(),
        )
        .limit(SHOW_EPISODES + 1)
        .tuples()
    )
    truncated = len(rows) > SHOW_EPISODES
    scanned = rows[:SHOW_EPISODES]
    cards = {str(row[0]): _episode_card(row) for row in scanned}
    ids = [row[0] for row in scanned]
    if not ids:
        return jsonify(
            {
                "q": query,
                "hits": [],
                "total": 0,
                "more": False,
                "scope": {"episodes": 0, "truncated": False},
            }
        )
    hits, more = _segment_hits(ids, words, limit)
    return jsonify(
        {
            "q": query,
            "hits": _attach_episodes(hits, cards),
            "total": len(hits),
            "more": more,
            "scope": {"episodes": len(ids), "truncated": truncated},
        }
    )


@transcribe_api.route("/transcripts/search")
@login_required
def podcast_search():
    _rate(search_limiter)
    limit = _int_arg("limit", SEARCH_DEFAULT, 1, SEARCH_MAX)
    found = _words(request.args.get("q"))
    values = settings.load(_app())
    budget = int(values["search_segments"] or 0)
    if not found:
        return jsonify(
            {
                "q": "",
                "hits": [],
                "total": 0,
                "more": False,
                "scope": {
                    "episodes": 0,
                    "segments": 0,
                    "truncated": False,
                    "limit_segments": budget,
                },
            }
        )
    words, query = found
    rows = list(
        PodcastTranscript.select(
            PodcastTranscript.episode,
            PodcastTranscript.n_segments,
            PodcastEpisode.title,
            PodcastEpisode.publish_date,
            PodcastEpisode.created,
            PodcastEpisode.image_md5,
            PodcastChannel.id,
            PodcastChannel.title,
            PodcastChannel.cover_art_md5,
            PodcastChannel.gone,
        )
        .join(PodcastEpisode, on=(PodcastEpisode.id == PodcastTranscript.episode))
        .join(PodcastChannel, on=(PodcastChannel.id == PodcastEpisode.channel))
        .where(PodcastTranscript.status == "ready")
        .order_by(
            fn.COALESCE(PodcastEpisode.publish_date, PodcastEpisode.created).desc(),
            PodcastTranscript.updated.desc(),
        )
        .limit(LIBRARY_EPISODES + 1)
        .tuples()
    )
    truncated = len(rows) > LIBRARY_EPISODES
    cards = {}
    ids = []
    segments = 0
    for row in rows[:LIBRARY_EPISODES]:
        count = int(row[1] or 0)
        if budget and ids and segments + count > budget:
            truncated = True
            break
        eid = str(row[0])
        cards[eid] = _episode_card(row)
        ids.append(row[0])
        segments += count
    if not ids:
        return jsonify(
            {
                "q": query,
                "hits": [],
                "total": 0,
                "more": False,
                "scope": {
                    "episodes": 0,
                    "segments": 0,
                    "truncated": truncated,
                    "limit_segments": budget,
                },
            }
        )
    hits, more = _segment_hits(ids, words, limit)
    return jsonify(
        {
            "q": query,
            "hits": _attach_episodes(hits, cards),
            "total": len(hits),
            "more": more,
            "scope": {
                "episodes": len(ids),
                "segments": segments,
                "truncated": truncated,
                "limit_segments": budget,
            },
        }
    )
