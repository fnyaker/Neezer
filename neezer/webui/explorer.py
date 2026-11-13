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
* ``POST|GET /api/explorer/discover`` — for one parent style and a discovery
  level 1..5, look a few Deezer playlists up and MEASURE a handful of their
  previews with the library's own analyser (``deezer.explorer.discover_style``).
  The thresholds tighten as the level goes down and never accept a cousin.

The discovery takes seconds (Deezer calls, a download, ffmpeg), so it never
runs in a request thread: the first call answers ``state: pending`` and starts
one worker; the screen polls every 2 s. The answer is kept in ``Meta`` for a
day, so it survives a restart and a poll never asks Deezer again. It is a
BULK/PREFETCH job: reniced, throttled, at most two playlists and eight
previews, with a pause between calls — it gives way to anyone listening.

``Meta`` is a 32-character key with a 256-character value, so the cache key is
``explorer_style:<blake2b8 of the folded style>:<level>`` and the JSON is split
over numbered chunk rows beside a small pointer row.
"""

from __future__ import annotations

import hashlib
import json
import logging
import threading
import time
from urllib.parse import urlsplit

from flask import Blueprint, jsonify, request

from .. import discover, genretree
from ..db import Meta, db
from deezerpy.errors import is_transport_failure

from . import _dz_live, login_required

logger = logging.getLogger(__name__)

explorer_api = Blueprint("explorer_api", __name__)

#: The result survives a day; a provider failure only long enough not to hammer
#: Deezer from a screen polling every two seconds.
CACHE_PREFIX = "explorer_style:"
CACHE_TTL = 24 * 3600
PROVIDER_TTL = 60
#: A chunk that fits ``Meta.value`` (CharField(256)) with room for the JSON
#: escaping; the pointer row holds the count and the write time.
CHUNK = 240
LEVELS = (1, 2, 3, 4, 5)

#: The discovery worker gives way to a person listening or downloading, and is
#: reniced: it is background work even though its result is awaited.
STYLE_DUTY = 0.5
STYLE_LISTEN_QUIET = 20.0
STYLE_MAX_LOAD = 0.8
#: At most this many discoveries run at once, whatever how many screens ask:
#: the rule is a bounded number of Deezer calls, not one per person per poll.
STYLE_WORKERS = 2

_inflight = {}
_inflight_lock = threading.Lock()


# -- the same same-origin guard as the rest of /api ---------------------------
# This is a separate blueprint (so it can be registered without touching
# webui/__init__.py), and the blueprint-level hook there does not reach it.
# The check is deliberately the same: a cross-site POST carrying the session
# cookie is refused; Fetch metadata first, Origin as the fallback.
@explorer_api.before_request
def reject_cross_site():
    if request.method in ("GET", "HEAD", "OPTIONS"):
        return
    site = request.headers.get("Sec-Fetch-Site")
    if site is not None:
        if site not in ("same-origin", "none"):
            return jsonify({"error": "cross-site request rejected"}), 403
        return
    origin = request.headers.get("Origin")
    if origin and urlsplit(origin).netloc != request.host:
        return jsonify({"error": "cross-site request rejected"}), 403


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
        age = time.time() - float(pointer.get("at") or 0)
    except (TypeError, ValueError):
        return None
    if age < 0 or age > ttl:
        return None
    return data


def _cache_write(style, level, payload):
    key = _cache_key(style, level)
    text = json.dumps(payload, ensure_ascii=False, separators=(",", ":"))
    chunks = [text[i : i + CHUNK] for i in range(0, len(text), CHUNK)] or [""]
    rows = [{"key": key, "value": json.dumps({"n": len(chunks), "at": time.time()})}]
    rows += [{"key": f"{key}:{i:02d}", "value": chunk} for i, chunk in enumerate(chunks)]
    with db.atomic():
        Meta.delete().where(Meta.key.startswith(key)).execute()
        Meta.insert_many(rows, fields=[Meta.key, Meta.value]).execute()


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


def _start(app, provider, style, level):
    """Start one worker for this (style, level), unless one already runs."""
    key = (style, level)
    with _inflight_lock:
        running = _inflight.get(key)
        if running is not None and running.is_alive():
            return False
        live = sum(1 for t in _inflight.values() if t.is_alive())
        if live >= STYLE_WORKERS:
            return False
        thread = threading.Thread(
            target=_run, args=(app, provider, style, level),
            name="explorer-style", daemon=True,
        )
        _inflight[key] = thread
        thread.start()
        return True


def _request_args():
    """``(style node, level, error response)`` from the body or the query."""
    if request.method == "POST":
        data = request.get_json(silent=True)
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


def _discover(style, level):
    cached = _cache_read(style, level)
    if cached is not None:
        return jsonify(cached)
    from flask import current_app

    provider = _dz_live()
    if provider is None:
        # Not a verdict about the style: the screen says "Deezer injoignable".
        return jsonify(_payload(style, level, [], error="provider"))
    _start(current_app._get_current_object(), provider, style, level)
    return jsonify(_payload(style, level, [], state="pending"))


@explorer_api.route("/explorer/discover", methods=["POST"])
@login_required
def explorer_discover():
    """Body ``{"style": "<tree name>", "level": 1..5}``; pending or ready."""
    style, level, error = _request_args()
    if error:
        return error
    return _discover(style, level)


@explorer_api.route("/explorer/discover")
@login_required
def explorer_discover_state():
    """The same answer as the POST, for the 2-second poll."""
    style, level, error = _request_args()
    if error:
        return error
    return _discover(style, level)
