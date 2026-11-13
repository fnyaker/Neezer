# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""The Explorer: the parent styles one person actually listens to, and Deezer
playlists that may hold the next title of one of them.

Two routes, both read-only on this server and on Deezer:

* ``GET /api/explorer/styles`` — the ten parent styles of the caller's own
  taste (``discover.parent_styles``: raw verdicts carried to their nearest
  meaningful parent, merged, ranked). A leaf is never served as a style: a
  person who plays Boogie lives in Disco/funk.
* ``POST /api/explorer/discover`` — for one parent style and a discovery level
  1..5, look a few Deezer playlists up and MEASURE a handful of their previews
  with the library's own analyser (``deezer.explorer.discover_style``). The
  thresholds tighten as the level goes down and never accept a cousin.

The discovery takes seconds (Deezer calls, a download, ffmpeg), so it never
runs in a request thread: the POST answers ``state: pending`` and starts one
worker; the screen polls every 2 s. The answer is kept in ``Meta`` for a day, so
it survives a restart and a poll never asks Deezer again. It is a
BULK/PREFETCH job: reniced, throttled, at most two playlists and eight
previews, with a pause between calls — it gives way to anyone listening.

``GET`` NEVER starts work (it is the poller): with no cache and no worker it
answers ``state: idle``. Only a POST starts a discovery, and only a POST that
actually starts one spends the person's budget (6 / 10 min, 30 / h) — a cached
answer and a poll are free. When both workers are busy the answer is ``503``
with ``Retry-After``, never a ``pending`` that nobody is working on.

``Meta`` is a 32-character key with a 256-character value, so the cache key is
``explorer_style:<blake2b8 of the folded style>:<level>`` and the JSON is split
over numbered chunk rows beside a small pointer row. A write prunes the expired
groups and caps the cache at ``CACHE_MAX`` groups, so a busy server never grows
tens of thousands of rows.
"""

from __future__ import annotations

import hashlib
import json
import logging
import math
import re
import threading
import time

from flask import Blueprint, jsonify, request

from .. import discover, genretree
from ..db import Meta, db
from deezerpy.errors import is_transport_failure

from . import _dz_live, login_required, same_origin_guard

logger = logging.getLogger(__name__)

explorer_api = Blueprint("explorer_api", __name__)

#: The result survives a day; a provider failure only long enough not to hammer
#: Deezer from a screen polling every two seconds.
CACHE_PREFIX = "explorer_style:"
CACHE_TTL = 24 * 3600
PROVIDER_TTL = 60
#: A chunk that fits ``Meta.value`` (CharField(256)) with room for the JSON
#: escaping; the pointer row holds the count, the write time and the TTL.
CHUNK = 240
LEVELS = (1, 2, 3, 4, 5)
#: The cache is a bounded number of (pointer + chunks) groups. A discovery is
#: a few KB and a busy screen can ask for many styles; without this cap the
#: 256-char rows would add up to tens of thousands over months.
CACHE_MAX = 512
_POINTER_RE = re.compile(r"\A[0-9a-f]{8}:[1-5]\Z")

#: A POST body is two short fields. Refuse anything bigger BEFORE parsing it.
MAX_BODY = 8 * 1024

#: The discovery worker gives way to a person listening or downloading, and is
#: reniced: it is background work even though its result is awaited.
STYLE_DUTY = 0.5
STYLE_LISTEN_QUIET = 20.0
STYLE_MAX_LOAD = 0.8
#: At most this many discoveries run at once, whatever how many screens ask:
#: the rule is a bounded number of Deezer calls, not one per person per poll.
STYLE_WORKERS = 2
#: What one person may START: a short burst and an hourly ceiling. A cached
#: answer, a poll and an already-running discovery are free.
BUDGET_WINDOWS = ((10 * 60, 6), (3600, 30))

_inflight = {}
_inflight_lock = threading.Lock()
_budget = {}  # user id -> deque of monotonic start times
_budget_lock = threading.Lock()


# -- the same same-origin guard as the rest of /api ---------------------------
@explorer_api.before_request
def reject_cross_site():
    return same_origin_guard()


@explorer_api.errorhandler(Exception)
def _json_error(exc):
    from werkzeug.exceptions import HTTPException

    if isinstance(exc, HTTPException):
        return jsonify({"error": exc.name}), exc.code
    logger.error("Unhandled error on %s %s", request.method, request.path, exc_info=True)
    return jsonify({"error": "server error"}), 500


# -- the parent styles --------------------------------------------------------


@explorer_api.route("/explorer/styles")
@login_required
def explorer_styles():
    """The ten parent styles of this person's taste, with their scale.

    Reads the cached taste profile (120 s) and the genre tree; no provider is
    asked. An account with no listening and no analysed title gets an empty
    list and a ``source`` that says so — the library's styles are never passed
    off as their taste.
    """
    return jsonify(discover.parent_styles(request.webuser, discover.STYLES_TOP))


# -- the per-style discovery --------------------------------------------------


def _cache_key(style, level):
    """``explorer_style:<8 hex>:<level>`` — 25 characters, inside Meta's 32."""
    digest = hashlib.blake2b(genretree.fold(style).encode("utf-8"), digest_size=4).hexdigest()
    return f"{CACHE_PREFIX}{digest}:{level}"


def _cache_read(style, level):
    key = _cache_key(style, level)
    rows = {
        r.key: r.value
        for r in Meta.select(Meta.key, Meta.value).where(Meta.key.startswith(key))
    }
    pointer_raw = rows.get(key)
    if not pointer_raw:
        return None
    try:
        pointer = json.loads(pointer_raw)
        n = int(pointer.get("n") or 0)
        text = "".join(rows.get(f"{key}:{i:02d}", "") for i in range(n))
        data = json.loads(text)
    except (TypeError, ValueError):
        return None
    if not isinstance(data, dict) or data.get("style") != style or data.get("level") != level:
        return None
    ttl = PROVIDER_TTL if data.get("error") else CACHE_TTL
    try:
        ttl = float(pointer.get("ttl") or ttl)
        age = time.time() - float(pointer.get("at") or 0)
    except (TypeError, ValueError):
        return None
    if age < 0 or age > ttl:
        return None
    return data


def _cache_write(style, level, payload):
    key = _cache_key(style, level)
    ttl = PROVIDER_TTL if payload.get("error") else CACHE_TTL
    text = json.dumps(payload, ensure_ascii=False, separators=(",", ":"))
    chunks = [text[i : i + CHUNK] for i in range(0, len(text), CHUNK)] or [""]
    rows = [{
        "key": key,
        "value": json.dumps({"n": len(chunks), "at": time.time(), "ttl": ttl}),
    }]
    rows += [{"key": f"{key}:{i:02d}", "value": chunk} for i, chunk in enumerate(chunks)]
    with db.atomic():
        Meta.delete().where(Meta.key.startswith(key)).execute()
        Meta.insert_many(rows, fields=[Meta.key, Meta.value]).execute()
    try:
        _prune_cache()
    except Exception:  # a full cache must never lose the answer just written
        logger.warning("explorer: could not prune the style cache", exc_info=True)


def _prune_cache():
    """Drop expired groups (and orphan chunks), then cap the cache at CACHE_MAX.

    Runs after a write, on the worker thread. A group is a pointer row plus its
    numbered chunk rows; deleting one deletes both, so no half-payload survives
    to be parsed by a poll.
    """
    rows = {
        r.key: r.value
        for r in Meta.select(Meta.key, Meta.value).where(Meta.key.startswith(CACHE_PREFIX))
    }
    now = time.time()
    pointers = {}
    for key, value in rows.items():
        if not _POINTER_RE.match(key[len(CACHE_PREFIX):]):
            continue
        try:
            pointer = json.loads(value)
            at = float(pointer.get("at") or 0)
            ttl = float(pointer.get("ttl") or CACHE_TTL)
        except (TypeError, ValueError):
            at, ttl = 0.0, CACHE_TTL
        pointers[key] = (at, ttl)

    drop = {key for key, (at, ttl) in pointers.items() if now - at > ttl}
    live = sorted(
        ((at, key) for key, (at, _ttl) in pointers.items() if key not in drop),
        reverse=True,
    )
    if len(live) > CACHE_MAX:
        drop.update(key for _at, key in live[CACHE_MAX:])
    # A chunk whose pointer is gone (a write interrupted mid-transaction) is
    # unreadable forever: clean it with the rest.
    for key in rows:
        if key in pointers:
            continue
        parent = key.rsplit(":", 1)[0]
        if parent in drop or parent not in pointers:
            drop.add(key)
    for key in list(drop):
        if key in pointers:
            drop.update(k for k in rows if k.startswith(key + ":"))
    keys = list(drop)
    for i in range(0, len(keys), 400):
        Meta.delete().where(Meta.key.in_(keys[i : i + 400])).execute()
    return len(keys)


def _payload(style, level, playlists=None, error=None, state="ready"):
    out = {"state": state, "style": style, "level": level, "playlists": playlists or []}
    if error:
        out["error"] = error
    return out


def _error_name(exc, provider):
    """Deezer unreachable (a provider failure) versus this server failing."""
    if is_transport_failure(exc):
        return "provider"
    probe = getattr(provider, "available", None)
    try:
        if probe is not None and not probe():
            return "provider"
    except Exception:
        pass
    return "failed"


# -- budgets and workers ------------------------------------------------------


def _budget_take(uid):
    """Record one discovery START; return ``(allowed, retry_after_seconds)``.

    ``retry_after`` is how long until the oldest start that fills a window
    expires, so the client can honour ``Retry-After`` exactly. Only a start
    that creates a worker calls this; a GET, a cache hit and a request that
    finds a worker already running do not.
    """
    key = str(uid)
    now = time.monotonic()
    with _budget_lock:
        stamps = _budget.setdefault(key, [])
        horizon = now - BUDGET_WINDOWS[-1][0]
        while stamps and stamps[0] < horizon:
            stamps.pop(0)
        retry = 0.0
        for window, cap in BUDGET_WINDOWS:
            inside = [t for t in stamps if t >= now - window]
            if len(inside) >= cap:
                retry = max(retry, inside[0] + window - now)
        if retry > 0:
            return False, max(1, int(math.ceil(retry)))
        stamps.append(now)
        return True, 0


def _busy_response(status, message, seconds):
    seconds = max(1, int(seconds))
    response = jsonify({"error": message, "retry_after": seconds})
    response.status_code = status
    response.headers["Retry-After"] = str(seconds)
    return response


def _running(style, level):
    with _inflight_lock:
        thread = _inflight.get((style, level))
        return thread is not None and thread.is_alive()


def _run(app, provider, style, level):
    """The worker: one discovery, then the cache, then it leaves."""
    from ..db import close_connection, open_connection
    from ..deezer import explorer as X
    from ..deezer import workload

    payload = None
    try:
        with app.app_context():
            open_connection(reuse=True)
            try:
                gate = workload.Throttle(
                    duty=STYLE_DUTY, listen_quiet=STYLE_LISTEN_QUIET, max_load=STYLE_MAX_LOAD
                )
                workload.renice(10)
                try:
                    playlists = X.discover_style(provider, style, level, gate)
                    payload = _payload(style, level, playlists)
                except Exception as exc:
                    payload = _payload(
                        style, level, [], error=_error_name(exc, provider)
                    )
                    logger.info("explorer: discovery of %s (level %s) stopped: %s",
                                style, level, exc)
                _cache_write(style, level, payload)
            finally:
                close_connection()
    except Exception:
        # The cache itself failed (a database problem): still answer this run,
        # the next poll will simply ask again.
        logger.warning("explorer: could not cache the discovery of %s", style, exc_info=True)
    finally:
        with _inflight_lock:
            _inflight.pop((style, level), None)


def _start(app, provider, style, level, uid):
    """``(state, retry_after)``: start one worker if one may be started.

    ``running`` means a worker for this very key already exists (a poll/POST
    can answer ``pending``); ``saturated`` means both slots are busy (503, never
    a pending nobody is working on); ``budget`` means this person started too
    many (429); ``started`` is a new worker, and only then is the budget spent.
    """
    key = (style, level)
    with _inflight_lock:
        running = _inflight.get(key)
        if running is not None and running.is_alive():
            return "running", 0
        live = sum(1 for t in _inflight.values() if t.is_alive())
        if live >= STYLE_WORKERS:
            return "saturated", 5
        allowed, retry = _budget_take(uid)
        if not allowed:
            return "budget", retry
        thread = threading.Thread(
            target=_run, args=(app, provider, style, level),
            name="explorer-style", daemon=True,
        )
        _inflight[key] = thread
        thread.start()
        return "started", 0


# -- the request --------------------------------------------------------------


def _read_bounded(limit):
    """At most ``limit`` bytes from a body whose length was not declared."""
    chunks, remaining = [], limit
    while remaining > 0:
        chunk = request.stream.read(min(64 * 1024, remaining))
        if not chunk:
            break
        chunks.append(chunk)
        remaining -= len(chunk)
    return b"".join(chunks)


def _post_data():
    """``(dict, error_response)`` without ever parsing a body over MAX_BODY."""
    length = request.content_length
    if length is not None and length > MAX_BODY:
        return None, (jsonify({"error": "body too large"}), 413)
    if length is None:
        raw = _read_bounded(MAX_BODY + 1)
        if len(raw) > MAX_BODY:
            return None, (jsonify({"error": "body too large"}), 413)
        try:
            data = json.loads(raw.decode("utf-8"))
        except (UnicodeDecodeError, ValueError):
            return None, (jsonify({"error": "invalid body"}), 400)
        return data, None
    return request.get_json(silent=True), None


def _request_args():
    """``(style node, level, error response)`` from the body or the query."""
    if request.method == "POST":
        data, error = _post_data()
        if error:
            return None, None, error
        if not isinstance(data, dict):
            return None, None, (jsonify({"error": "invalid body"}), 400)
        raw_style, raw_level = data.get("style"), data.get("level")
    else:
        raw_style = request.args.get("style")
        raw_level = request.args.get("level")
    if not isinstance(raw_style, str) or not raw_style.strip():
        return None, None, (jsonify({"error": "unknown style"}), 400)
    style = genretree.get().node(raw_style.strip()[:64])
    if style is None:
        return None, None, (jsonify({"error": "unknown style"}), 400)
    if isinstance(raw_level, bool):
        return None, None, (jsonify({"error": "invalid level"}), 400)
    try:
        if isinstance(raw_level, float) and not raw_level.is_integer():
            raise ValueError
        level = int(raw_level)
    except (TypeError, ValueError):
        return None, None, (jsonify({"error": "invalid level"}), 400)
    if level not in LEVELS:
        return None, None, (jsonify({"error": "invalid level"}), 400)
    return style, level, None


def _cached_or_none(style, level):
    cached = _cache_read(style, level)
    return jsonify(cached) if cached is not None else None


def _get_state(style, level):
    """The poller: cache, else a live worker, else idle. Never starts anything."""
    answer = _cached_or_none(style, level)
    if answer is not None:
        return answer
    if _running(style, level):
        return jsonify(_payload(style, level, [], state="pending"))
    return jsonify(_payload(style, level, [], state="idle"))


def _post_discover(style, level):
    answer = _cached_or_none(style, level)
    if answer is not None:
        return answer
    from flask import current_app

    provider = _dz_live()
    if provider is None:
        # Not a verdict about the style: the screen says "Deezer injoignable".
        return jsonify(_payload(style, level, [], error="provider"))
    state, retry = _start(
        current_app._get_current_object(), provider, style, level, request.webuser.id
    )
    if state in ("started", "running"):
        return jsonify(_payload(style, level, [], state="pending"))
    if state == "saturated":
        return _busy_response(503, "busy", retry)
    return _busy_response(429, "too many discoveries", retry)


@explorer_api.route("/explorer/discover", methods=["POST", "GET"])
@login_required
def explorer_discover():
    """GET polls (and never starts); POST starts a discovery.

    Body ``{"style": "<tree name>", "level": 1..5}`` on POST; query string on
    GET. Answers ``pending``/``ready``/``idle`` (GET only), or 429/503 with
    ``Retry-After`` when this person or the server is at its limit.
    """
    style, level, error = _request_args()
    if error:
        return error
    if request.method == "POST":
        return _post_discover(style, level)
    return _get_state(style, level)
