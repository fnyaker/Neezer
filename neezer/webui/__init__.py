# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Custom Deezer-native JSON API for the bundled discovery web UI.

Session-cookie authenticated (so the SPA isn't tied to the Subsonic protocol's
limits). Discovery comes straight from the Deezer gateway via DeezerProvider;
playback reuses the import + archive + cache machinery. Album/artist artwork is
returned as Deezer CDN URLs the browser loads directly.
"""

from __future__ import annotations

import logging
import math
import os.path
import re
import sys
import threading
import datetime
import json
import time
import uuid
from concurrent.futures import ThreadPoolExecutor
from functools import wraps
from urllib.parse import urlsplit

from werkzeug.security import safe_join

from flask import (
    Blueprint,
    current_app,
    g,
    has_request_context,
    jsonify,
    redirect,
    request,
    send_file,
    session,
)
import peewee
from peewee import fn

from ..db import (
    Album,
    Artist,
    Meta,
    Playlist,
    PlaylistMember,
    PlaylistPlacement,
    PlaylistTrack,
    PodcastChannel,
    PodcastEpisode,
    PodcastMarker,
    PodcastProgress,
    PodcastTranscript,
    StarredTrack,
    Track,
    TrackArtist,
    User,
    db,
    now,
)
from deezerpy.errors import is_transport_failure

from ..deezer.provider import TrackUnavailable
from ..managers.user import UserManager
from ..ratelimit import auth_limiter
from .. import access, playlist_access as placcess, radio, similar
from . import refs
from ..utils import like_term
from .refs import cover_url_for

logger = logging.getLogger(__name__)

webapi = Blueprint("webapi", __name__)


# Same-origin guard for every state-changing /api call. The SPA is same-origin,
# so a cross-site POST is by definition not us: this stops a hostile page from
# driving the API with the visitor's session cookie, and it's the layer that
# still holds if SameSite is ignored (old browsers, or a cookie replayed by an
# embedded webview). Fetch metadata is preferred; Origin is the fallback for
# clients that don't send Sec-Fetch-*.
#
# One function, installed on each /api blueprint: the Explorer has its own
# blueprint, and a per-blueprint copy would drift from this one.
def same_origin_guard():
    if request.method in ("GET", "HEAD", "OPTIONS"):
        return None
    site = request.headers.get("Sec-Fetch-Site")
    if site is not None:
        if site not in ("same-origin", "none"):
            return jsonify({"error": "cross-site request rejected"}), 403
        return None
    origin = request.headers.get("Origin")
    if origin:
        # Compare hosts only: a TLS-terminating proxy that isn't declared with
        # proxy_fix_hops makes the app see http:// while the browser sends an
        # https:// Origin, and that mismatch is a deployment artifact, not an
        # attack (a hostile origin always differs by host).
        if urlsplit(origin).netloc != request.host:
            return jsonify({"error": "cross-site request rejected"}), 403
    return None


@webapi.before_request
def reject_cross_site():
    return same_origin_guard()


# A person waiting in front of the app outranks every background job.
#
# This server answers from a small fixed pool of threads, and a first play of a
# Deezer track holds one of them for as long as the FLAC takes to arrive. The
# client prefetches through that same route, so a handful of prefetches used to
# take the WHOLE pool — and then the gains, the lyrics and the favourites list
# queued behind them for half a minute each. Nothing was slow; everything was
# waiting.
#
# So a request that nobody is looking at says so (``X-NS-Background: 1``, set by
# the prefetcher and the cache warmers) and is held to a fraction of the pool.
# Over that fraction it is REFUSED rather than queued — a background request
# that waits is still holding the thread it was meant not to hold — and the
# client simply tries again later. Foreground requests are never gated, so the
# rest of the pool is always there for somebody who is waiting.
def _is_background_request() -> bool:
    if request.headers.get("X-NS-Background") == "1":
        return True
    return request.args.get("bg") == "1"


def _count_stream_response(view):
    @wraps(view)
    def wrapped(*args, **kwargs):
        from ..deezer import workload

        return workload.counted_response(
            view(*args, **kwargs), background=_is_background_request()
        )

    return wrapped


@webapi.before_request
def _admit_by_priority():
    from ..deezer import workload

    request.ns_background = _is_background_request()
    request.ns_slot = False
    if not request.ns_background:
        return
    if not workload.admit_background():
        # Deliberately 503 + Retry-After and not a queue: the client reads this
        # as "not now", which is exactly what it is.
        resp = jsonify({"error": "busy", "background": True})
        resp.status_code = 503
        resp.headers["Retry-After"] = "5"
        return resp
    request.ns_slot = True


@webapi.teardown_request
def _release_priority_slot(_exc=None):
    if getattr(request, "ns_slot", False):
        from ..deezer import workload

        request.ns_slot = False
        workload.release_background()


# Last-resort safety net. Every route below already handles the failures it can
# foresee, but Deezer is a third party: it invents new error shapes, hands back
# half a JSON document, or throws a database race at us mid-import. None of that
# is worth an HTML stack trace the SPA can't parse — the app must stay up and the
# one broken call must fail cleanly, with the traceback in the log for us.
@webapi.errorhandler(Exception)
def _unhandled_api_error(exc):
    from werkzeug.exceptions import HTTPException

    if isinstance(exc, HTTPException):
        original = exc.get_response()
        resp = jsonify({"error": exc.name})
        resp.status_code = exc.code
        for header in ("Content-Range", "Retry-After"):
            if header in original.headers:
                resp.headers[header] = original.headers[header]
        return resp
    logger.error("Unhandled error on %s %s", request.method, request.path, exc_info=True)
    return jsonify({"error": "server error"}), 500


_CDN = "https://e-cdns-images.dzcdn.net/images/{kind}/{md5}/{w}x{w}-000000-80-0-0.jpg"


# -- helpers ----------------------------------------------------------------


def _real_md5(md5):
    """An art id worth a CDN URL. Deezer spells "no art" as all zeroes, and a
    302 to that JPEG is what made every Sillon fallback disappear behind the
    same generic sleeve."""
    value = str(md5 or "").strip()
    return value if value and set(value) != {"0"} else None


def _image(kind, md5, size=500):
    md5 = _real_md5(md5)
    return _CDN.format(kind=kind, md5=md5, w=size) if md5 else None


def _title(t):
    title = t.get("SNG_TITLE", "") or ""
    version = t.get("VERSION") or ""
    return f"{title} {version}".strip() if version and version not in title else title


# Deezer's per-credit ROLE_ID; anything unrecognised is a feature, not a drop.
_ROLE_NAMES = {"0": "Main", "5": "Featured"}


def _credit_list(t: dict) -> list[dict]:
    """Credited artists for a raw gateway track, in the label's own order.

    ``ARTISTS`` is the gateway's full credit list, but it isn't on every
    response (the trimmed favourites payload has only ART_ID/ART_NAME), so an
    absent list falls back to the single primary artist. Callers can therefore
    always read `artists` and never have to special-case it.
    """
    raw = t.get("ARTISTS")
    primary = {
        "deezer_id": str(t.get("ART_ID") or ""),
        "name": t.get("ART_NAME", "") or "",
        "role": "Main",
    }
    if not isinstance(raw, list) or not raw:
        return [primary] if primary["deezer_id"] else []

    def order(e):
        try:
            return int(e.get("ARTISTS_SONGS_ORDER") or 0)
        except (TypeError, ValueError):
            return 0

    out, seen = [], set()
    for e in sorted((x for x in raw if isinstance(x, dict)), key=order):
        aid = str(e.get("ART_ID") or "")
        name = e.get("ART_NAME", "") or ""
        if not aid or not name or aid in seen:
            continue  # Deezer lists an artist twice when they hold two roles
        seen.add(aid)
        out.append(
            {
                "deezer_id": aid,
                "name": name,
                "role": _ROLE_NAMES.get(str(e.get("ROLE_ID")), "Featured"),
            }
        )
    return out or ([primary] if primary["deezer_id"] else [])


def _display_artist(artists: list[dict], fallback: str = "") -> str:
    """The credit line: "A", or "A feat. B, C"."""
    if not artists:
        return fallback
    main = [a["name"] for a in artists if a.get("role") == "Main"]
    feat = [a["name"] for a in artists if a.get("role") != "Main"]
    # Nobody credited as Main (an unmapped ROLE_ID, a compilation): list
    # everyone plainly rather than promoting the first one and then repeating
    # them after a "feat." that has no primary to hang off.
    if not main:
        return ", ".join(feat) or fallback
    return f"{', '.join(main)} feat. {', '.join(feat)}" if feat else ", ".join(main)


def _gain(raw):
    """Per-track ReplayGain (dB) as a float for the web player's normalization,
    or None. Deezer sends it as a string on the gateway, a number on the API."""
    if raw in (None, ""):
        return None
    try:
        return float(raw)
    except (TypeError, ValueError):
        return None


def _track(t):
    if not t or not t.get("SNG_ID"):
        return None
    expl = str(t.get("EXPLICIT_LYRICS", ""))
    artists = _credit_list(t)
    return {
        "deezer_id": str(t.get("SNG_ID")),
        "title": _title(t),
        "duration": int(t.get("DURATION") or 0),
        "added": int(t.get("DATE_ADD") or 0),
        "explicit": expl == "1",
        "gain": _gain(t.get("GAIN")),
        # `artist` stays the primary one — every existing caller reads it.
        # `artists` is the full credit list and `display_artist` the ready-made
        # "A feat. B" line, so the UI never has to assemble it itself.
        "artist": {"deezer_id": str(t.get("ART_ID")), "name": t.get("ART_NAME", "")},
        "artists": artists,
        "display_artist": _display_artist(artists, t.get("ART_NAME", "") or ""),
        "album": {
            "deezer_id": str(t.get("ALB_ID")),
            "title": t.get("ALB_TITLE", ""),
            "cover": _image("cover", t.get("ALB_PICTURE")),
        },
    }


def _album(a):
    if not a:
        return None
    return {
        "deezer_id": str(a.get("ALB_ID")),
        "title": a.get("ALB_TITLE") or a.get("TITLE", ""),
        "cover": _image("cover", a.get("ALB_PICTURE")),
        "artist": {"deezer_id": str(a.get("ART_ID")), "name": a.get("ART_NAME", "")},
        "nb_tracks": a.get("NUMBER_TRACK"),
        "year": (str(a.get("PHYSICAL_RELEASE_DATE") or a.get("DIGITAL_RELEASE_DATE") or "")[:4]) or None,
        # The gw album page reports whether the account already favorited it, so
        # the detail page can show the correct heart state (and avoid a "re-add"
        # that reads as a fresh favorite). Absent on older responses → False.
        "is_favorite": bool(a.get("FAVORITE_STATUS")),
    }


def _artist(ar):
    if not ar:
        return None
    return {
        "deezer_id": str(ar.get("ART_ID")),
        "name": ar.get("ART_NAME", ""),
        "picture": _image("artist", ar.get("ART_PICTURE")),
        "nb_fan": ar.get("NB_FAN"),
    }


def _tracks(items):
    return flag_unavailable([x for x in (_track(t) for t in (items or [])) if x])


# Tracks we already know can't be played are flagged in every list we serve, so
# the UI can mark them and offer a replacement instead of letting you find out
# by pressing play. One batched query per list — never one per track.
_FLAG_CHUNK = 400


def flag_unavailable(items):
    ids = [str(t["deezer_id"]) for t in items if t and t.get("deezer_id")]
    if not ids:
        return items
    dead = set()
    try:
        for start in range(0, len(ids), _FLAG_CHUNK):
            chunk = ids[start : start + _FLAG_CHUNK]
            dead.update(
                str(row.deezer_id)
                for row in Track.select(Track.deezer_id).where(
                    Track.deezer_id.in_(chunk) & Track.unavailable.is_null(False)
                )
            )
    except Exception:  # a badge is never worth failing the response over
        logger.debug("Unavailability lookup failed", exc_info=True)
        return items
    if dead:
        for t in items:
            if t and str(t.get("deezer_id")) in dead:
                t["unavailable"] = True
    return items


def _playlist(p):
    if not p:
        return None
    pid = p.get("PLAYLIST_ID")
    if not pid:
        return None
    kind = (p.get("PICTURE_TYPE") or "playlist").lower()
    return {
        "deezer_id": str(pid),
        "title": p.get("TITLE", ""),
        "description": p.get("DESCRIPTION") or "",
        "cover": _image(kind, p.get("PLAYLIST_PICTURE")),
        "nb_tracks": p.get("NB_SONG"),
        "duration": int(p.get("DURATION") or 0),
        "owner": p.get("PARENT_USERNAME") or p.get("PARENT_USER", ""),
        "is_favorite": bool(p.get("IS_FAVORITE")),
    }


# -- normalizers for the public REST API (api.deezer.com) -------------------
# The public API is more stable than the private gateway for search/charts and
# returns ready-to-use image URLs. Shapes differ (lowercase REST), so these map
# them to the SAME JSON the gateway normalizers above produce.


def _pic(d, *keys):
    for k in keys:
        if d.get(k):
            return d[k]
    return None


def _track_api(t):
    if not t or not t.get("id"):
        return None
    alb = t.get("album") or {}
    art = t.get("artist") or {}
    # The public API calls the credit list `contributors` and already carries a
    # role per entry ("Main" / "Featured"), so it maps straight across.
    artists, seen = [], set()
    for c in t.get("contributors") or []:
        if not isinstance(c, dict):
            continue
        cid = str(c.get("id") or "")
        name = c.get("name") or ""
        if not cid or not name or cid in seen:
            continue
        seen.add(cid)
        artists.append({"deezer_id": cid, "name": name, "role": c.get("role") or "Main"})
    if not artists and art.get("id"):
        artists = [
            {"deezer_id": str(art.get("id")), "name": art.get("name", ""), "role": "Main"}
        ]
    return {
        "deezer_id": str(t.get("id")),
        "title": t.get("title") or t.get("title_short") or "",
        "duration": int(t.get("duration") or 0),
        "explicit": bool(t.get("explicit_lyrics")),
        "gain": _gain(t.get("gain")),
        "artist": {"deezer_id": str(art.get("id")), "name": art.get("name", "")},
        "artists": artists,
        "display_artist": _display_artist(artists, art.get("name", "")),
        "album": {
            "deezer_id": str(alb.get("id")),
            "title": alb.get("title", ""),
            "cover": _pic(alb, "cover_medium", "cover_big", "cover"),
        },
    }


def _album_api(a):
    if not a or not a.get("id"):
        return None
    art = a.get("artist") or {}
    return {
        "deezer_id": str(a.get("id")),
        "title": a.get("title", ""),
        "cover": _pic(a, "cover_medium", "cover_big", "cover"),
        "artist": {"deezer_id": str(art.get("id")), "name": art.get("name", "")},
        "nb_tracks": a.get("nb_tracks"),
        "year": (str(a.get("release_date") or "")[:4]) or None,
    }


def _artist_api(a):
    if not a or not a.get("id"):
        return None
    return {
        "deezer_id": str(a.get("id")),
        "name": a.get("name", ""),
        "picture": _pic(a, "picture_medium", "picture_big", "picture"),
        "nb_fan": a.get("nb_fan"),
    }


def _playlist_api(p):
    if not p or not p.get("id"):
        return None
    user = p.get("user") or {}
    return {
        "deezer_id": str(p.get("id")),
        "title": p.get("title", ""),
        "description": p.get("description") or "",
        "cover": _pic(p, "picture_medium", "picture_big", "picture"),
        "nb_tracks": p.get("nb_tracks"),
        "owner": user.get("name", ""),
        "is_favorite": False,
    }


def login_required(f):
    @wraps(f)
    def wrapper(*args, **kwargs):
        uid = session.get("uid")
        if not uid:
            return jsonify({"error": "unauthorized"}), 401
        try:
            user = User[uid]
        except (User.DoesNotExist, ValueError):
            session.clear()
            return jsonify({"error": "unauthorized"}), 401
        # Signed cookies aren't revocable; the epoch is. A password change or a
        # role change bumps it, which invalidates every cookie minted before.
        if session.get("epoch", 0) != (user.session_epoch or 0):
            session.clear()
            return jsonify({"error": "unauthorized"}), 401
        if user.disabled:
            session.clear()
            return jsonify({"error": "unauthorized"}), 401
        request.webuser = user
        return f(*args, **kwargs)

    return wrapper


# The single Deezer account (the ARL) belongs to the admin / sync user. Only the
# admin sees that account's personal data — its playlists, favorites, Flow and
# recommendations — and only the admin's plays feed Deezer telemetry. Everyone
# else is a guest: Deezer is just a content catalogue (search / browse / play),
# their favorites are private and local-only, and the account is never mutated.
def _is_admin():
    u = getattr(request, "webuser", None)
    return bool(u and u.admin)


def admin_required(f):
    """Reject non-admins outright — for endpoints that would write to the
    shared Deezer account (favorites, playlist edits, Flow tuning)."""

    @wraps(f)
    def wrapper(*args, **kwargs):
        if not _is_admin():
            return jsonify({"error": "forbidden"}), 403
        return f(*args, **kwargs)

    return wrapper


# -- upload ownership -------------------------------------------------------
# A Track with a non-NULL owner is somebody's private upload; NULL means it is
# part of the shared library (scanned music folders, Deezer-backed rows) and
# stays visible to everyone. Guests used to see — and stream, and download —
# each other's uploaded files, because nothing tied a Track to a user at all.


def _visible_track_clause():
    """Peewee expression: shared-library tracks + the caller's own uploads."""
    user = getattr(request, "webuser", None)
    if user is not None and user.admin:
        return None  # admins see everything
    if user is None:  # pragma: no cover - login_required runs first
        return Track.owner.is_null(True)
    return Track.owner.is_null(True) | (Track.owner == user.id)


def _visible_tracks(query):
    clause = _visible_track_clause()
    return query if clause is None else query.where(clause)


def _may_access_track(track) -> bool:
    """Whether the caller may read this specific track."""
    if track is None:
        return False
    owner_id = getattr(track, "owner_id", None)
    if owner_id is None:
        return True
    user = getattr(request, "webuser", None)
    return bool(user and (user.admin or user.id == owner_id))


#: Which Deezer feature each route uses Deezer for (plan multi-provider §15):
#: a person who may not use it gets the route's local answer, exactly as when
#: Deezer is switched off. A route not listed uses Deezer for catalogue pages
#: ("browse"); None is not gated: administration, health, and the routes that
#: only ever touch rows the library already holds (a cover, a verdict check).
DEEZER_FEATURE = {
    "search": "search",
    "replacement_candidates": "search",
    "genre_references": "search",
    "search_podcasts": "podcasts",
    "subscribe_podcast": "podcasts",
    "import_playlist_start": "links",
    "track_credits": "track_info",
    # these archive what they touch
    "stream": "stream",
    "download": "stream",
    "favorite": "stream",
    "favorite_entity": "stream",
    "create_playlist": "stream",
    "add_playlist_tracks": "stream",
    "share_waveform": "stream",
    "share_file": "stream",
    "share_clip": "stream",
    "export_zip": "stream",
    "genre_reference_import": "stream",
    # not gated
    "login": None,
    "me": None,
    "get_settings": None,
    "set_settings": None,
    "deezer_status": None,
    "trigger_sync": None,
    "report_listen": None,
    "cover": None,
    "services_status": None,
    "services_export": None,
    "probe_track": None,
    "replace_track": None,
    "delete_track": None,
    "genre_share_import": None,
    # collaborative playlists: the database alone, no provider anywhere in the
    # path (an unlisted route defaults to "browse" and would be gated on a
    # provider grant for nothing).
    "playlist_version": None,
    "playlist_members": None,
    "playlist_invite": None,
    "playlist_member_role": None,
    "playlist_member_remove": None,
    "playlist_leave": None,
    # the database's own credits: no Deezer behind them
    "person_credits": None,
    "credits_backfill": None,
    "onboarding_get": None,
    "onboarding_artists": None,
    "onboarding_search": None,
    "onboarding_picture": None,
    "onboarding_like": None,
    "onboarding_unlike": None,
    "onboarding_related": None,
    "onboarding_artists_done": None,
    "onboarding_account_done": None,
    "onboarding_feed_retry": None,
    "onboarding_account_options": None,
    "onboarding_account_link": None,
    "onboarding_account_import": None,
}


def _deezer_feature():
    endpoint = (request.endpoint or "").rpartition(".")[2]
    return DEEZER_FEATURE.get(endpoint, "browse")


def _deezer_allowed() -> bool:
    """May the person behind this request use Deezer for what this route
    does? Outside a request (a worker) there is nobody to ask about."""
    if not has_request_context():
        return True
    cached = getattr(g, "_dz_allowed", None)
    if cached is not None:
        return cached
    feature = _deezer_feature()
    user = getattr(request, "webuser", None)
    allowed = True if feature is None or user is None else access.can(user, "deezer", feature)
    g._dz_allowed = allowed
    return allowed


_UNSET = object()


def _provider():
    """The Deezer that serves this request (lot P3): the person's own account
    when it can, else the server's, else one lent to them. None when none
    can — and then ``_deezer_anonymous()`` says whether the public API may
    still answer for them."""
    if not has_request_context():
        return getattr(current_app, "deezer", None)
    cached = getattr(g, "_dz_provider", _UNSET)
    if cached is not _UNSET:
        return cached
    feature = _deezer_feature()
    user = getattr(request, "webuser", None)
    if feature is None:
        provider, anonymous = getattr(current_app, "deezer", None), False
    else:
        from ..deezer import accounts as dz_accounts

        provider, anonymous = dz_accounts.serving(current_app._get_current_object(), user, feature)
    g._dz_provider = provider
    g._dz_anonymous = provider is None and anonymous
    return provider


def _deezer_anonymous() -> bool:
    """Is this request served by Deezer's public API alone (no account for
    this person, the anonymous use granted)?"""
    if not has_request_context():
        return False
    _provider()
    return bool(getattr(g, "_dz_anonymous", False))


# A failure that is about DEEZER, not about this server, carries a code of its
# own in `X-NS-Error`. The client reads it as final: retrying is the server's
# job — every call already retries inside its network budget, and the circuit
# breaker probes Deezer again once its cooldown is over — so a client that
# re-asked three times only paid three more answers that it already had (the
# same instant refusal, ~3 s of backoff, then the page saying "not found").
NO_RETRY_HEADER = "X-NS-Error"
DEEZER_UNREACHABLE = "deezer-unreachable"
DEEZER_DISABLED = "deezer-disabled"
DEEZER_DENIED = "deezer-denied"


def _deezer_disabled():
    if _deezer_anonymous():
        # Only the public API serves this person, and this page needs an account.
        resp = jsonify(
            {
                "error": "Cette page demande un compte Deezer, et aucun ne vous est prêté",
                "code": DEEZER_DENIED,
            }
        )
        resp.status_code = 403
        resp.headers[NO_RETRY_HEADER] = DEEZER_DENIED
        return resp
    if getattr(current_app, "deezer", None) is not None and not _deezer_allowed():
        # Deezer runs, but not for this person: a refusal, not an outage.
        resp = jsonify(
            {"error": "Vous n'avez pas accès à Deezer sur ce serveur", "code": DEEZER_DENIED}
        )
        resp.status_code = 403
        resp.headers[NO_RETRY_HEADER] = DEEZER_DENIED
        return resp
    resp = jsonify({"error": "Deezer proxy disabled", "code": DEEZER_DISABLED})
    resp.status_code = 503
    resp.headers[NO_RETRY_HEADER] = DEEZER_DISABLED
    return resp


def _deezer_unreachable():
    """503 for a page only Deezer has, while Deezer cannot be reached."""
    provider = _provider()
    outage = (getattr(provider, "outage", lambda: None)() or {}) if provider else {}
    # Bounded: an "until further notice" backoff is infinite, and Infinity is
    # not JSON any browser will parse.
    retry_in = min(float(outage.get("retry_in") or 0), 3600.0)
    resp = jsonify(
        {
            "error": "Deezer est injoignable",
            "code": DEEZER_UNREACHABLE,
            "reason": "network",  # the vocabulary of /deezer/status
            "retry_in": round(retry_in, 1),
        }
    )
    resp.status_code = 503
    resp.headers[NO_RETRY_HEADER] = DEEZER_UNREACHABLE
    resp.headers["Retry-After"] = str(max(1, int(retry_in + 0.999)) if retry_in else 30)
    return resp


def _deezer_down(exc=None) -> bool:
    """Was a missing answer Deezer not answering (rather than Deezer saying no)?

    `exc` is what the route's call raised, if it made one. A route that did not
    call at all (``_dz_live`` said an outage is known) passes nothing, and the
    provider's own instant reading decides.
    """
    provider = _provider()
    if provider is None:
        return False
    if exc is not None and is_transport_failure(exc):
        return True
    probe = getattr(provider, "available", None)
    return probe is not None and not probe()


def _deezer_missing(exc=None):
    """The answer for a page neither Deezer nor the database could give."""
    if _provider() is None:
        if getattr(g, "_dz_public", False):
            # The public API was asked for this person, and did not have it.
            if exc is not None and is_transport_failure(exc):
                return _deezer_unreachable()
            return jsonify({"error": "not found"}), 404
        return _deezer_disabled()
    if _deezer_down(exc):
        return _deezer_unreachable()
    return jsonify({"error": "not found"}), 404


def _need_provider():
    provider = _provider()
    if provider is None:
        return None, _deezer_disabled()
    return provider, None


def _log_deezer_failure(message, *args, exc=None):
    """Log the Deezer call that just failed, from inside its ``except``.

    With a traceback for a real error — and WITHOUT one when Deezer is simply
    not answering. During an outage every request of every user walks this path,
    and a stack trace apiece turns somebody else's downtime into gigabytes of
    our logs (and, on a small server, a full disk). One line each is plenty:
    the interesting fact is that Deezer is down, and it is the same fact 10 000
    times over.
    """
    if exc is None:
        exc = sys.exc_info()[1]
    if is_transport_failure(exc):
        logger.warning("%s (Deezer unreachable: %s)", message % args if args else message, exc)
    else:
        logger.warning(message, *args, exc_info=exc or True)


def _dz_api():
    """The Deezer public-API client, or None when it can't be reached — never raises.

    Reading ``provider.dz`` LOGS IN on first use, so merely touching it throws
    while Deezer is down; several routes read it outside their try block, which
    turned a Deezer outage into a 500 and skipped the local fallback they
    already had two lines below. It also answers instantly (no network) for as
    long as a known outage lasts, so a route degrades to the library at once
    instead of queueing behind a call that is going to fail.
    """
    provider = _dz_live()
    if provider is None:
        if _deezer_anonymous():
            # Nobody's account serves this person: the public API, signed in
            # as nobody, answers what it answers to anybody (lot P3).
            from ..deezer.accounts import public_api

            g._dz_public = True
            return public_api()
        return None
    try:
        return provider.dz.api
    except Exception:
        logger.debug("Deezer client unavailable", exc_info=True)
        return None


def _dz_live():
    """The provider, but only while Deezer is worth calling. Never raises.

    Same idea as ``_dz_api`` for the routes that go through the provider's own
    methods: it lets a route drop to its local-database answer without first
    making (and waiting on) a call already known to fail.
    """
    provider = _provider()
    if provider is None:
        return None
    # A provider that can't tell us (an alternative backend, a test double) is
    # treated as reachable — the same rule the health check itself follows:
    # never report an outage we have not actually observed.
    probe = getattr(provider, "available", None)
    if probe is not None and not probe():
        return None
    return provider


# A Deezer id is a plain positive integer. `str.isdigit()` was far too loose:
# it accepts the whole Unicode Nd category and superscripts, so "١٢٣", "²" and
# (after the lstrip) "-5" all passed and were concatenated into outgoing
# api.deezer.com URLs.
_DEEZER_ID_RE = re.compile(r"\A[0-9]{1,20}\Z")


def _valid_id(value):
    """A Deezer numeric id (track/album/artist/playlist). Rejects junk early."""
    return bool(_DEEZER_ID_RE.fullmatch(str(value or "")))


# -- upload quota -----------------------------------------------------------
# Non-admins may only upload up to a per-user byte quota; admins are unlimited.
# The GB figure is admin-settable at runtime (stored in the Meta KV table, which
# overrides the config default) and each user's files live in their own upload
# subfolder, so usage is just the size of what's on disk — it self-corrects and
# never drifts from a running counter.

_QUOTA_META_KEY = "upload_quota_gb"


def _quota_gb() -> float:
    """The per-user upload quota in GB (0 = unlimited). DB value wins over config."""
    row = Meta.get_or_none(Meta.key == _QUOTA_META_KEY)
    if row is not None:
        try:
            return max(0.0, float(row.value))
        except (TypeError, ValueError):
            pass
    try:
        return max(0.0, float(current_app.config["WEBAPP"].get("upload_quota_gb") or 0))
    except (TypeError, ValueError):
        return 0.0


def _set_quota_gb(value: float) -> None:
    value = max(0.0, float(value))
    # Store whole numbers without a trailing ".0" so the UI shows "5", not "5.0".
    s = str(int(value)) if value == int(value) else repr(value)
    row = Meta.get_or_none(Meta.key == _QUOTA_META_KEY)
    if row is None:
        Meta.create(key=_QUOTA_META_KEY, value=s)
    else:
        row.value = s
        row.save()


def _user_upload_dir(archive_dir: str, user) -> str:
    """Where one user's uploaded files live — one folder per user, for accounting."""
    return os.path.join(archive_dir, "Uploads", str(user.id))


def _user_upload_usage(archive_dir: str, user) -> int:
    """Total bytes this user has uploaded (sum of files under their upload dir)."""
    total = 0
    for dirpath, _dirs, files in os.walk(_user_upload_dir(archive_dir, user)):
        for fn in files:
            try:
                total += os.path.getsize(os.path.join(dirpath, fn))
            except OSError:
                pass
    return total


_UPLOAD_BAD = re.compile(r'[/\\:*?"<>|\x00-\x1f\x7f]')


def _safe_upload_name(raw: str):
    """``(filename, extension)`` for an uploaded file — Unicode-safe.

    werkzeug's ``secure_filename`` ASCII-folds, which **erases** a name written
    in a non-Latin script entirely: ``Песня.mp3`` came back as ``mp3`` and
    ``曲.flac`` as ``flac``, so the extension check below then rejected a
    perfectly good file as an unsupported format. Anyone whose library isn't in
    Latin script simply couldn't upload.

    So we do the dangerous-character removal ourselves and keep the rest of the
    name, and we read the extension from the ORIGINAL string so it survives
    whatever the sanitiser does to the stem. The result can contain no path
    separator, no ``..`` and no control character, so it is safe to join onto
    the destination directory.
    """
    raw = str(raw or "")
    ext = os.path.splitext(raw)[1][1:].lower()
    # Last path component under EITHER separator (a Windows client sends "\").
    stem = raw.replace("\\", "/").rsplit("/", 1)[-1]
    if ext:
        stem = stem[: -(len(ext) + 1)]
    stem = _UPLOAD_BAD.sub("_", stem)
    # No dotfiles, no "..", and no trailing dot/space (Windows drops those,
    # which would silently collide two distinct uploads).
    stem = stem.strip().lstrip(".").rstrip(". ")
    # Filesystems cap a component at 255 bytes; leave room for a " (n)" suffix.
    stem = stem.encode("utf-8")[:180].decode("utf-8", "ignore").strip(" .")
    if not stem or not ext:
        return "", ext
    return f"{stem}.{ext}", ext


def _stream_size(stream) -> int:
    """Byte length of an uploaded file, measured (not trusted from the client)."""
    pos = stream.tell()
    stream.seek(0, os.SEEK_END)
    size = stream.tell()
    stream.seek(pos)
    return size


# -- local (non-Deezer) tracks ----------------------------------------------
# Files imported from the archive directory have no Deezer id; their universal
# id (for streaming, etc.) is the Track UUID, and they carry `local: true` so
# the UI can show a "not on Deezer" badge.


# -- what a track's own file is ---------------------------------------------
# The "Qualité" column of every list. A payload may only claim what the row's
# own FILE says; before the archive, ``Track.bitrate`` is the NOMINAL rate of
# the quality that was REQUESTED (``provider.NOMINAL_BITRATE``), not what
# anybody hears, and the path's extension is not settled either (Deezer may
# answer with MP3_320 for a track asked in FLAC). So an unarchived row says
# nothing at all rather than lie.
#
# Read from columns the list already carries: the suffix is the row's ``path``,
# the rate is ``bitrate``, "archived" is ``last_modification`` (0 until the
# archive stamps it, and reset to 0 by the only code that deletes one). No file
# is stat'ed and no query is added — ``tools/perf_api.py`` shows the same
# statement count for every list.
_QUALITY_NAMES = {
    "flac": "FLAC", "alac": "ALAC", "wav": "WAV", "wv": "WavPack",
    "ape": "APE", "aif": "AIFF", "aiff": "AIFF", "aifc": "AIFF", "dsf": "DSD",
    "opus": "OPUS", "ogg": "VORBIS", "oga": "VORBIS",
    "m4a": "AAC", "mp4": "AAC", "aac": "AAC", "mp3": "MP3", "mp2": "MP2",
}
# Same set as ``providers.quality``'s: a lossless file's rate is not a
# listening quality, so its label carries the format alone ("FLAC").
_LOSSLESS_SUFFIXES = frozenset(
    {"flac", "alac", "wav", "wv", "ape", "aif", "aiff", "aifc", "dsf"}
)


def _quality_fields(path, bitrate, on_disk: bool) -> dict:
    """``suffix``, ``bitrate`` and ``quality`` of a row whose file is here.

    Short label, the way the player shows it: ``FLAC``, ``OPUS 320``,
    ``MP3 320``. A lossy file whose rate is unknown keeps its format alone
    (``MP3``), never a rate nobody measured.
    """
    suffix = os.path.splitext(path or "")[1][1:].lower()
    if not on_disk or not suffix:
        return {}
    out = {"suffix": suffix}
    rate = int(bitrate or 0)
    if rate > 0:
        out["bitrate"] = rate
    name = _QUALITY_NAMES.get(suffix) or suffix.upper()
    if suffix in _LOSSLESS_SUFFIXES or not rate:
        out["quality"] = name
    else:
        out["quality"] = f"{name} {rate}"
    return out


def _local_track(t: Track) -> dict:
    out = {
        "id": str(t.id),
        "deezer_id": str(t.id),  # the universal id for local tracks is the UUID
        "local": True,
        "title": t.title,
        "duration": t.duration or 0,
        "release_date": t.release_date,
        "explicit": False,
        "unavailable": t.unavailable is not None,
        "gain": t.gain,
        "artist": {"deezer_id": str(t.artist.id), "name": t.artist.name},
        # An uploaded file has a single tagged artist; keep the shape uniform so
        # the UI never has to special-case local tracks.
        "artists": [
            {"deezer_id": str(t.artist.id), "name": t.artist.name, "role": "Main"}
        ],
        "display_artist": t.artist.name,
        "album": {
            "deezer_id": str(t.album.id),
            "title": t.album.name,
            "cover": cover_url_for(t.album.id),
        },
    }
    if t.youtube_id:
        # YouTube keeps its own label (play width, or the archived file's
        # measured rate): `_youtube_fields` owns `bitrate`, and a generic
        # `quality` here would replace "OPUS 133" with a bare "OPUS".
        _youtube_fields(out, t.youtube_id, t.bitrate)
    else:
        # A local row IS its file: the upload endpoint and the scan both import
        # the file that is already on disk.
        out.update(_quality_fields(t.path, t.bitrate, True))
    return out


def _youtube_fields(out: dict, video_id, bitrate) -> dict:
    """What marks a YouTube fallback track in the API: its source and the
    bitrate its file REALLY has (0 until it is archived) — the player shows it
    rather than the quality that was asked for. Its art goes through
    ``/api/cover``, which falls back to the search's thumbnail before the
    file (and its embedded art) is here."""
    out["source"] = "youtube"
    out["youtube_id"] = video_id
    out["bitrate"] = int(bitrate or 0)
    out["album"]["cover"] = "/api/cover/" + out["deezer_id"]
    # No artist or album page to open: YouTube's names are labels here, and a
    # link would land on "introuvable". An empty id is what the SPA reads as
    # "not a link" (ArtistLine, the track menu, the album column).
    out["artist"]["deezer_id"] = ""
    out["album"]["deezer_id"] = ""
    for a in out["artists"]:
        a["deezer_id"] = ""
    return out


def _not_from_podcasts():
    """A Track query's "not a podcast episode" clause.

    Episodes are ``PodcastEpisode`` rows, never user files — but a scan older
    than that rule may have created Track rows pointing at their audio (they
    have no Deezer id, so they looked exactly like uploads). Those must not be
    listed in "Mes fichiers" or the offline search any more; their audio stays
    exactly where it is (nothing here deletes anything).

    One clause, so a list keeps its statement count: the episodes' own paths as
    a subquery, and a NOT LIKE on the podcast directory when the archive is
    configured — both run by the database inside the list's own SELECT.
    """
    clause = ~Track.path.in_(
        PodcastEpisode.select(PodcastEpisode.path).where(
            PodcastEpisode.path.is_null(False)
        )
    )
    archive_dir = (current_app.config.get("DEEZER") or {}).get("archive_dir")
    if archive_dir:
        from ..deezer.library import PODCAST_DIR_NAME, podcast_root_path

        # Both spellings: the absolute one the app writes everywhere, and the
        # raw config-relative one an older scan could have stored.
        roots = {
            podcast_root_path(archive_dir),
            os.path.normpath(os.path.join(archive_dir, PODCAST_DIR_NAME)),
        }
        for root in roots:
            clause = clause & ~Track.path.startswith(root.rstrip(os.sep) + os.sep)
    return clause


def _local_search_tracks(query: str, limit: int) -> list:
    """Search the on-device library so downloaded music is findable even when
    Deezer is unreachable. Covers uploaded local files (deezer_id NULL, always on
    disk) *and* archived Deezer tracks (file present) — anything playable without
    a Deezer call. Imported-but-not-yet-archived rows are skipped: they'd need
    Deezer to stream, so they surface through the live Deezer search instead.
    """
    like = like_term(query)
    if like is None:
        return []
    # "On disk" is a column: `last_modification` is 0 until the archive stamps
    # it, and reset to 0 by the only code that deletes one (cleanup.py). It used
    # to be a stat() per row over five times `limit` rows sorted by title — and
    # a common word whose first two hundred matches were merely imported hid
    # every archived match behind them.
    q = _visible_tracks(
        Track.select(Track, Album, Artist)
        .join(Album)
        .switch(Track)
        .join(Artist)
        .where(
            (
                Track.title.contains(like)
                | Album.name.contains(like)
                | Artist.name.contains(like)
            )
            & (
                (Track.deezer_id.is_null(True) & Track.youtube_id.is_null(True))
                | (Track.last_modification > 0)
            )
        )
    )
    return _db_tracks(q.where(_not_from_podcasts()).order_by(Track.title).limit(limit))


def _on_disk_tracks():
    """The visible tracks that play without asking anyone: uploads and archives."""
    return _visible_tracks(
        Track.select(Track.id).where(
            (Track.deezer_id.is_null(True) & Track.youtube_id.is_null(True))
            | (Track.last_modification > 0)
        )
    )


def _local_search_entities(query: str, limit: int) -> tuple:
    """Artists and albums of the library whose NAME matches, each holding at
    least one title on disk: what the search shows while Deezer is out (the
    catalogue's artists and albums are its own answer otherwise). Two
    statements, plus the album grid's count."""
    like = like_term(query)
    if like is None:
        return [], []
    held = _on_disk_tracks()
    artists = [
        {
            "id": _uuid_text(rid),
            "deezer_id": str(dz or ""),
            "name": name,
            "picture": f"/api/cover/{_uuid_text(rid)}" if dz else None,
        }
        for rid, name, dz in (
            Artist.select(_raw(Artist.id), Artist.name, Artist.deezer_id)
            .where(
                Artist.name.contains(like)
                & Artist.id.in_(held.select(Track.artist))
            )
            .order_by(Artist.name)
            .limit(limit)
            .tuples()
        )
    ]
    albums = _db_album_cards(
        Album.select(Album, Artist)
        .join(Artist)
        .where(Album.name.contains(like) & Album.id.in_(held.select(Track.album)))
        .order_by(Album.name)
        .limit(limit)
    )
    return artists, albums


def _local_starred() -> list:
    """The current user's locally-starred (non-Deezer) tracks."""
    return list(
        Track.select(Track, Album, Artist)
        .join(Album)
        .switch(Track)
        .join(Artist)
        .switch(Track)
        .join(StarredTrack, on=(StarredTrack.starred == Track.id))
        .where(StarredTrack.user == request.webuser, Track.deezer_id.is_null(True))
    )


def _user_starred_query():
    """Every track the current user has starred (local *and* Deezer), newest
    first, as a query (``_db_tracks`` reads a query as tuples). Used for guests,
    whose favorites are private/local-only — there's no Deezer-account
    favorites list to read from — and for the admin's offline fallback."""
    return (
        Track.select(Track, Album, Artist)
        .join(Album)
        .switch(Track)
        .join(Artist)
        .switch(Track)
        .join(StarredTrack, on=(StarredTrack.starred == Track.id))
        .where(StarredTrack.user == request.webuser)
        .order_by(StarredTrack.date.desc())
    )


def _user_starred() -> list:
    """``_user_starred_query`` as model rows."""
    return list(_user_starred_query())


def _primary_credit(t: Track) -> list[dict]:
    """The credit list of a track that has no credit rows — rows imported before
    the feature existed, or tracks Deezer only ever described with one artist."""
    return [
        {
            "deezer_id": str(t.artist.deezer_id or t.artist.id),
            "name": t.artist.name,
            "role": "Main",
        }
    ]


def _db_credits(t: Track) -> list[dict]:
    """Credited artists for ONE DB track. Serialising a list goes through
    ``_db_tracks`` instead — this query per row is an N+1 on a big page."""
    rows = (
        TrackArtist.select(TrackArtist, Artist)
        .join(Artist)
        .where(TrackArtist.track == t.id)
        .order_by(TrackArtist.position)
    )
    out = [
        {
            "deezer_id": str(r.artist.deezer_id or r.artist.id),
            "name": r.artist.name,
            "role": r.role,
        }
        for r in rows
    ]
    return out or _primary_credit(t)


# SQLite's default parameter limit is 999; stay well under it when expanding an
# id list into an IN (...) clause.
_IN_CHUNK = 400


def _credits_by_track(tracks: list) -> dict:
    """``{track id: [credit dicts]}`` for a whole page in one round trip.

    A 4000-track favourites list would otherwise run 4000 credit queries — the
    exact N+1 that makes a big list crawl. Only tracks that actually have credit
    rows appear; callers fall back to ``_primary_credit`` for the rest.
    """
    tids = [t.id for t in tracks if t.deezer_id is not None]
    out: dict = {}
    for i in range(0, len(tids), _IN_CHUNK):
        rows = (
            TrackArtist.select(TrackArtist, Artist)
            .join(Artist)
            .where(TrackArtist.track << tids[i : i + _IN_CHUNK])
            .order_by(TrackArtist.track, TrackArtist.position)
        )
        for r in rows:
            out.setdefault(r.track_id, []).append(
                {
                    "deezer_id": str(r.artist.deezer_id or r.artist.id),
                    "name": r.artist.name,
                    "role": r.role,
                }
            )
    return out


def _db_tracks(rows) -> list:
    """Serialize a list of DB tracks, resolving every credit in one query.

    A QUERY (Track joined to Album and Artist, as every list here is built) is
    read as plain tuples by ``_db_track_rows``: the same dicts, without three
    model objects hydrated per row. Model rows already in hand go through
    ``_db_track`` one by one."""
    if isinstance(rows, peewee.ModelSelect):
        return _db_track_rows(rows)
    rows = list(rows)
    by_track = _credits_by_track(rows)
    # `.get(id, [])`, not `.get(id)`: an empty list says "already resolved, this
    # one simply has no credit rows". None would send it back to the per-row
    # query and undo the batching for every plain single-artist track.
    return [_db_track(t, by_track.get(t.id, [])) for t in rows]


# -- big lists, read as tuples --------------------------------------------------
# A 4000-track favourites page or playlist spent three quarters of its time in
# Peewee building a Track, an Album and an Artist object per row — parsing three
# UUIDs and a datetime, a setattr per column — only for ``_db_track`` to flatten
# them straight back into a dict. Measured on 8000 tracks: 405 ms for the
# favourites, 447 for a 4000-track playlist. Read as tuples of exactly the
# columns the JSON carries, the same answer takes a fraction of that; the
# output is byte-for-byte ``_db_track``'s (tests/test_webui.py compares them).


def _raw(field):
    """``field`` read without Peewee's per-row conversion: a UUID comes back as
    text (32 hex digits on SQLite and MySQL, dashed on Postgres) instead of
    being parsed into a ``uuid.UUID`` on every row. MySQL spells the cast
    CHAR."""
    target = getattr(db, "obj", db)
    return field.cast("CHAR" if isinstance(target, peewee.MySQLDatabase) else "TEXT")


def _uuid_text(v) -> str:
    """A raw UUID column as ``str(uuid.UUID(...))`` would print it."""
    if isinstance(v, uuid.UUID):
        return str(v)
    s = str(v)
    if len(s) == 32:
        return f"{s[:8]}-{s[8:12]}-{s[12:16]}-{s[16:20]}-{s[20:]}"
    return s


_ROW_COLUMNS = None


def _row_columns():
    global _ROW_COLUMNS
    if _ROW_COLUMNS is None:
        _ROW_COLUMNS = (
            _raw(Track.id),
            Track.deezer_id,
            Track.title,
            Track.duration,
            Track.unavailable.is_null(False),
            Track.gain,
            Track.youtube_id,
            Track.bitrate,
            Track.release_date,
            _raw(Artist.id),
            Artist.deezer_id,
            Artist.name,
            _raw(Album.id),
            Album.deezer_id,
            Album.name,
            Album.cover_md5,
            # What the row's own file is (see ``_quality_fields``): the path
            # carries the suffix, `last_modification` says archived. Same
            # SELECT, not one query more.
            Track.path,
            Track.last_modification,
        )
    return _ROW_COLUMNS


def _credits_by_raw_id(tids: list, within=None) -> dict:
    """``_credits_by_track`` keyed by the raw track id, read as tuples.

    `within` is the list's own query when it can stand in for the id list: one
    statement with a subquery instead of ten with four hundred parameters each
    (building those parameter lists was a third of what was left of a big
    list's time)."""
    out: dict = {}

    def read(cond):
        rows = (
            TrackArtist.select(
                _raw(TrackArtist.track), Artist.deezer_id, _raw(Artist.id), Artist.name, TrackArtist.role
            )
            .join(Artist)
            .where(cond)
            .order_by(TrackArtist.track, TrackArtist.position)
            .tuples()
        )
        for tid, adz, aid, name, role in rows:
            out.setdefault(tid, []).append(
                {"deezer_id": str(adz or _uuid_text(aid)), "name": name, "role": role}
            )

    if within is not None:
        read(TrackArtist.track.in_(within))
        return out
    for i in range(0, len(tids), _IN_CHUNK):
        read(TrackArtist.track << tids[i : i + _IN_CHUNK])
    return out


def _db_track_rows(query) -> list:
    """``_db_tracks`` for a query: the same dicts, from tuples."""
    rows = list(query.select(*_row_columns()).tuples())
    # The list's own selection as the credits' subquery — unordered, and only
    # without a LIMIT, which MySQL refuses inside IN (...).
    within = None
    if getattr(query, "_limit", None) is None and getattr(query, "_offset", None) is None:
        within = query.select(Track.id).order_by().where(Track.deezer_id.is_null(False))
    credits = _credits_by_raw_id([r[0] for r in rows if r[1] is not None], within)
    out = []
    for (
        tid, dz, title, duration, gone, gain, ytid, bitrate, released,
        aid, adz, aname, alid, aldz, alname, md5, path, modified,
    ) in rows:
        if dz is None:
            uid = _uuid_text(tid)
            artist_id = _uuid_text(aid)
            row = {
                "id": uid,
                "deezer_id": uid,
                "local": True,
                "title": title,
                "duration": duration or 0,
                "release_date": released,
                "explicit": False,
                "unavailable": bool(gone),
                "gain": gain,
                "artist": {"deezer_id": artist_id, "name": aname},
                "artists": [{"deezer_id": artist_id, "name": aname, "role": "Main"}],
                "display_artist": aname,
                "album": {
                    "deezer_id": _uuid_text(alid),
                    "title": alname,
                    "cover": cover_url_for(alid),
                },
            }
            if ytid:
                _youtube_fields(row, ytid, bitrate)
            else:
                row.update(_quality_fields(path, bitrate, True))
            out.append(row)
            continue
        artists = credits.get(tid) or [
            {"deezer_id": str(adz or _uuid_text(aid)), "name": aname, "role": "Main"}
        ]
        row = {
            "id": _uuid_text(tid),
            "deezer_id": str(dz),
            "title": title,
            "duration": duration or 0,
            "release_date": released,
            "explicit": False,
            "unavailable": bool(gone),
            "gain": gain,
            "artist": {"deezer_id": str(adz or ""), "name": aname},
            "artists": artists,
            "display_artist": _display_artist(artists, aname),
            "album": {
                "deezer_id": str(aldz or ""),
                "title": alname,
                "cover": cover_url_for(alid),
            },
        }
        row.update(_quality_fields(path, bitrate, bool(modified)))
        out.append(row)
    return out


def _db_track(t: Track, credits: list | None = None) -> dict:
    """Normalize a DB Track row to the API track shape. Local files go through
    ``_local_track``; Deezer-imported rows rebuild their cover from the stored
    ``cover_md5`` so no Deezer call is needed.

    `credits` is the batched lookup from ``_db_tracks``: a list (even an empty
    one) means already resolved, None means look this row up on its own."""
    if t.deezer_id is None:
        return _local_track(t)
    artists = _db_credits(t) if credits is None else (credits or _primary_credit(t))
    out = {
        "id": str(t.id),
        "deezer_id": str(t.deezer_id),
        "title": t.title,
        "duration": t.duration or 0,
        "release_date": t.release_date,
        "explicit": False,
        # Straight off the row — no extra query for the lists that come from the
        # database (your playlists, the offline browse pages).
        "unavailable": t.unavailable is not None,
        "gain": t.gain,
        "artist": {"deezer_id": str(t.artist.deezer_id or ""), "name": t.artist.name},
        "artists": artists,
        "display_artist": _display_artist(artists, t.artist.name),
        "album": {
            "deezer_id": str(t.album.deezer_id or ""),
            "title": t.album.name,
            "cover": cover_url_for(t.album.id),
        },
    }
    # Only once the file is here: until then `bitrate` is the nominal rate of
    # the quality that was ASKED for (see ``_quality_fields``).
    out.update(_quality_fields(t.path, t.bitrate, t.last_modification > 0))
    return out


# -- DB fallbacks for browse pages (work when Deezer is unreachable) ---------
# Everything imported/archived is a normal DB row, so an album/artist/mix page
# can be rebuilt straight from the DB — no Deezer call — for offline browsing.


def _db_album_card(alb: Album, stats: tuple | None = None) -> dict:
    """A compact album entry (grid card), matching ``_album_api``'s shape.
    `stats` is (track count, first year) from ``_db_album_cards``; without it
    this album is counted on its own (two queries)."""
    if stats is None:
        stats = (alb.tracks.count(), alb.tracks.select(fn.min(Track.year)).scalar())
    return {
        "id": str(alb.id),
        "deezer_id": str(alb.deezer_id or ""),
        "title": alb.name,
        "cover": cover_url_for(alb.id),
        "release_date": alb.release_date,
        "artist": {
            "id": str(alb.artist.id),
            "deezer_id": str(alb.artist.deezer_id or ""),
            "name": alb.artist.name,
        },
        "nb_tracks": stats[0] or 0,
        "year": stats[1] or None,
    }


def _db_album_cards(albums) -> list:
    """``_db_album_card`` for a whole grid, counted in one grouped query
    instead of two per album."""
    albums = list(albums)
    stats = {}
    ids = [a.id for a in albums]
    for i in range(0, len(ids), _IN_CHUNK):
        for aid, nb, year in (
            _visible_tracks(
                Track.select(Track.album, fn.COUNT(Track.id), fn.MIN(Track.year))
                .where(Track.album << ids[i : i + _IN_CHUNK])
            )
            .group_by(Track.album)
            .tuples()
        ):
            stats[aid] = (nb, year)
    # An album of somebody else's upload has no visible track: no card.
    return [
        _db_album_card(a, stats.get(a.id, (0, None)))
        for a in albums
        if a.deezer_id is not None or a.id in stats
    ]


def _db_album_tracks(alb: Album) -> list:
    q = _visible_tracks(
        Track.select(Track, Album, Artist)
        .join(Album)
        .switch(Track)
        .join(Artist)
        .where(Track.album == alb)
    ).order_by(Track.disc, Track.number)
    return _db_tracks(q)


def _db_album_response(alb: Album) -> dict:
    tracks = _db_album_tracks(alb)
    return {
        "album": {
            "id": str(alb.id),
            "deezer_id": str(alb.deezer_id or ""),
            "title": alb.name,
            "cover": cover_url_for(alb.id),
            "release_date": alb.release_date,
            "artist": {
                "id": str(alb.artist.id),
                "deezer_id": str(alb.artist.deezer_id or ""),
                "name": alb.artist.name,
            },
            "nb_tracks": len(tracks),
            "year": alb.tracks.select(fn.min(Track.year)).scalar() or None,
            "is_favorite": _starred("album", row=alb),
        },
        "tracks": tracks,
        # The page is the library's own: the part only Deezer has is asked
        # for separately (``/album/<id>/on-deezer``).
        "library": True,
    }


def _related_artists(ar: Artist, limit: int = 20) -> list:
    """The artists whose tracks sound closest to this one's, as cards: the
    stored neighbours (``similar.artist_neighbors``), read from the database
    alone and only those the caller may see (an upload is its owner's)."""
    near = similar.artist_neighbors(ar.id, limit)
    if not near:
        return []
    ids = [aid for aid, _ in near]
    seen = {
        r[0]
        for r in _visible_tracks(Track.select(Track.artist).where(Track.artist.in_(ids)))
        .distinct()
        .tuples()
    }
    rows = {a.id: a for a in Artist.select().where(Artist.id.in_(list(seen)))}
    return [
        {
            "id": str(a.id),
            "deezer_id": str(a.deezer_id or ""),
            "name": a.name,
            "picture": f"/api/cover/{a.id}" if a.deezer_id else None,
        }
        for a in (rows.get(aid) for aid in ids)
        if a is not None
    ]


def _db_artist_response(ar: Artist) -> dict:
    albums = (
        Album.select(Album, Artist)
        .join(Artist)
        .where(Album.artist == ar)
        .order_by(Album.name)
    )
    cards = _db_album_cards(albums)
    # A few of the artist's tracks as a stand-in "top" shelf (DB order).
    top_q = (
        _visible_tracks(
            Track.select(Track, Album, Artist)
            .join(Album)
            .switch(Track)
            .join(Artist)
            .where(Track.artist == ar)
        )
        .order_by(Track.play_count.desc())
        .limit(15)
    )
    return {
        "artist": {
            "id": str(ar.id),
            "deezer_id": str(ar.deezer_id or ""),
            "name": ar.name,
            # Deezer's picture is fetched (and kept) by the cover route; an
            # artist with no Deezer id has none.
            "picture": f"/api/cover/{ar.id}" if ar.deezer_id else None,
            "nb_fan": None,
            "is_favorite": _starred("artist", row=ar),
        },
        "bio": None,
        "top": _db_tracks(top_q),
        "albums": cards,
        "related": _related_artists(ar),
        "library": True,  # ``/artist/<id>/on-deezer`` completes it
        # How many tracks of the library credit this artist as anything but a
        # performer (composer, author, producer…): the page's "Crédits" tab
        # shows when there is one. One indexed count.
        "credited": _credited_count(ar.name),
    }


def _credited_count(name) -> int:
    from .. import trackcredits
    from ..db import TrackCredit

    key = trackcredits.name_key(name)
    if not key:
        return 0
    try:
        return (
            trackcredits.credited_query_many([key])
            .select(TrackCredit.track)
            .distinct()
            .count()
        )
    except Exception:  # the tab is a nicety: the page answers anyway
        logger.debug("credited count failed for %s", name, exc_info=True)
        return 0


def _deezer_starred():
    """The current user's starred *Deezer* tracks (from the DB), newest first,
    as a query (``_db_tracks`` reads it as tuples)."""
    return (
        Track.select(Track, Album, Artist)
        .join(Album)
        .switch(Track)
        .join(Artist)
        .switch(Track)
        .join(StarredTrack, on=(StarredTrack.starred == Track.id))
        .where(StarredTrack.user == request.webuser, Track.deezer_id.is_null(False))
        .order_by(StarredTrack.date.desc())
    )


# -- DB-backed playlists ----------------------------------------------------
# The web app's *user* playlists live in neezer's own Playlist/PlaylistTrack
# tables (ordered by PlaylistTrack.index), exactly like the Subsonic side. This
# lets a playlist mix Deezer tracks and purely-local files (deezer_id NULL), and
# gives reliable reordering. They are local only: nothing is written back to the
# Deezer account.


def _resolve_db_playlist(pid):
    """Return the DB Playlist for a UUID *or* a mappable Deezer numeric id,
    else None (a non-imported recommendation/editorial playlist)."""
    from ..deezer import ids

    sid = str(pid)
    try:
        return Playlist[uuid.UUID(sid)]
    except (ValueError, Playlist.DoesNotExist):
        pass
    if _valid_id(sid):
        try:
            return Playlist[ids.playlist_uuid(sid)]
        except Playlist.DoesNotExist:
            pass
        me = getattr(request, "webuser", None)
        if me is not None:
            own = Playlist.select().where(
                (Playlist.deezer_id == sid) & (Playlist.user == me)
            ).first()
            if own is not None:
                return own
        return Playlist.select().where(Playlist.deezer_id == sid).first()
    return None


def _playlist_guard(playlist_id, need="read"):
    """``(playlist, role, error_response)`` for a web playlist route.

    The matrix of ``playlist_access`` applied on the wire, once:

    * an unknown/malformed id and a playlist the caller cannot see at all are
      BOTH ``404 {"error": "not found"}`` — a real-but-foreign playlist must
      not be confirmed to exist;
    * ``need="member"`` (the collaborators list) also answers 404 for a public
      reader: a public link carries no member names at all;
    * ``need="edit"``/``"manage"`` answer ``403 {"error": "forbidden"}`` to a
      caller who demonstrably has the playlist but lacks the right (a viewer
      writing, an editor inviting) — admin override included.

    A catalogue/editorial Deezer playlist has no local row and is a 404 here:
    the routes that use this guard mutate or list something this server owns.
    """
    pl = _resolve_db_playlist(playlist_id)
    if pl is None:
        return None, None, (jsonify({"error": "not found"}), 404)
    user = getattr(request, "webuser", None)
    role = placcess.role_for(pl, user)
    if role is None:
        return None, None, (jsonify({"error": "not found"}), 404)
    is_admin = bool(getattr(user, "admin", False))
    if need == "member":
        if role == "public":
            return None, None, (jsonify({"error": "not found"}), 404)
    elif need == "edit":
        if not (is_admin or placcess.role_can_edit(role)):
            return None, None, (jsonify({"error": "forbidden"}), 403)
    elif need == "manage":
        if not (is_admin or placcess.role_can_manage(role)):
            return None, None, (jsonify({"error": "forbidden"}), 403)
    return pl, role, None


def _playlist_conflict(playlist, scope):
    """409 for an optimistic-concurrency mismatch, with the AUTHORITATIVE
    counters — never the 4 000-track state.

    The row moved under us, so the object in hand is stale by construction:
    the counters are re-read (one statement, error path only) and the body
    stays a few hundred bytes, which is what makes the client's one-GET
    resync cheap on a phone link.
    """
    fresh = (
        Playlist.select(
            Playlist.version,
            Playlist.members_version,
            Playlist.last_edited_by,
            Playlist.last_edited_at,
        )
        .where(Playlist.id == playlist.id)
        .first()
    )
    version = fresh.version if fresh is not None else playlist.version
    members_version = (
        fresh.members_version if fresh is not None else playlist.members_version
    )
    edited_by = fresh.last_edited_by if fresh is not None else playlist.last_edited_by
    edited_at = fresh.last_edited_at if fresh is not None else playlist.last_edited_at
    return (
        jsonify(
            {
                "error": "conflict",
                "scope": scope,
                "version": version,
                "members_version": members_version,
                "last_edited_by": edited_by,
                "last_edited_at": edited_at.isoformat() if edited_at else None,
            }
        ),
        409,
    )


def _db_playlist_track_rows(pl, user=None):
    """The playlist's Track rows in order, with Album+Artist eager-loaded —
    ``pl.get_tracks()`` lazy-loads both per track (2 extra queries per row,
    thousands on a big playlist).

    ``user`` applies ``Track.visible_clause`` (shared library + the person's
    own uploads, nothing for an admin): a shared playlist never becomes a side
    channel for somebody else's private upload. Callers that need the stored
    rows (an export of one's own, the Subsonic owner view) pass nobody.
    """
    query = (
        Track.select(Track, Album, Artist)
        .join(Album)
        .switch(Track)
        .join(Artist)
        .switch(Track)
        .join(PlaylistTrack, on=(PlaylistTrack.track == Track.id))
        .where(PlaylistTrack.playlist == pl)
        .order_by(PlaylistTrack.index)
    )
    if user is not None:
        clause = Track.visible_clause(user)
        if clause is not None:
            query = query.where(clause)
    return query


def _playlist_private_tracks(pl) -> int:
    """How many rows are somebody's private upload (invisible to others).

    Shown to the owner/admin only, so the sheet can say how many titles other
    collaborators will not see. One grouped statement, never a row loop.
    """
    return (
        PlaylistTrack.select(fn.COUNT(PlaylistTrack.id))
        .join(Track, on=(PlaylistTrack.track == Track.id))
        .where((PlaylistTrack.playlist == pl.id) & Track.owner.is_null(False))
        .scalar()
        or 0
    )


def _visible_playlist_rows(pl, user):
    """``[(PlaylistTrack.id, PlaylistTrack.index, Track.id, Track.deezer_id)]``
    in stored order, with the private uploads ``user`` may not see filtered
    out. ONE statement; the visibility decision is ``Track.readable_by``'s."""
    rows = list(
        PlaylistTrack.select(
            PlaylistTrack.id,
            PlaylistTrack.index,
            Track.id,
            Track.deezer_id,
            Track.owner,
        )
        .join(Track, on=(PlaylistTrack.track == Track.id))
        .where(PlaylistTrack.playlist == pl.id)
        .order_by(PlaylistTrack.index)
        .tuples()
    )
    out = []
    for ptid, index, tid, dz, owner in rows:
        hidden = owner is not None and not (
            getattr(user, "admin", False) or owner == user.id
        )
        if hidden:
            continue
        out.append((ptid, index, tid, dz))
    return out


def _visible_playlist_count(pl, user) -> int:
    """The caller's visible row count: ``nb_tracks`` on every playlist answer."""
    query = (
        PlaylistTrack.select(fn.COUNT(PlaylistTrack.id))
        .join(Track, on=(PlaylistTrack.track == Track.id))
        .where(PlaylistTrack.playlist == pl.id)
    )
    clause = Track.visible_clause(user)
    if clause is not None:
        query = query.where(clause)
    return query.scalar() or 0


def _db_playlist_cover(tracks: list) -> str | None:
    """Pick the first available album cover as the playlist's cover (Playlist
    rows don't store a Deezer picture md5)."""
    for t in tracks:
        cover = (t.get("album") or {}).get("cover")
        if cover:
            return cover
    return None


def _ensure_track_row(provider, deezer_id, root, cache):
    """Materialize (idempotently) the DB Track row for a Deezer track id, so a
    freshly-added Deezer track can be referenced by a PlaylistTrack. Reuses the
    same upsert path as the importer (deezer/library.upsert_track)."""
    from ..deezer import library

    data = provider.get_track_info(deezer_id)
    return library.upsert_track(data, root, provider.default_quality, cache=cache)


def _resolve_tracks(provider, raw_ids) -> list:
    """Map universal track ids to Track rows: a UUID is a local/DB track, a
    numeric id is a Deezer track whose row is created on demand.

    Rows that already exist are read in one go (``refs.resolve_tracks``); only an
    id the database does not know goes further. With no ``provider`` (Deezer
    switched off) an unknown Deezer id is simply left out.
    """
    from ..deezer import library
    from . import refs

    raw_ids = list(raw_ids)
    known = refs.resolve_tracks(raw_ids)
    root = None
    cache = library.ImportCache()
    out = []
    for raw, row in zip(raw_ids, known):
        if row is not None:
            out.append(row)
            continue
        sid = str(raw)
        if _valid_id(sid):
            if provider is None:
                continue
            if root is None:
                root = library.get_root_folder(provider.archive_dir)
            try:
                out.append(_ensure_track_row(provider, sid, root, cache))
            except Exception:
                logger.warning("could not materialize Deezer track %s", sid, exc_info=True)
            continue
        # A YouTube search hit: its row is made from the search's answer.
        row = youtube.materialize(sid)
        if row is not None and _may_access_track(row):
            out.append(row)
    return out


# -- auth -------------------------------------------------------------------


@webapi.route("/login", methods=["POST"])
def login():
    throttled = not current_app.testing
    data = request.get_json(silent=True) or request.form
    username = data.get("username")
    password = data.get("password")
    if throttled and auth_limiter.is_blocked_any(request.remote_addr, username):
        return jsonify({"error": "too many attempts, try again later"}), 429
    user = UserManager.try_auth(username, password) if username and password else None
    if user is None or user.disabled:
        logger.error(
            "Failed web login for user %s (IP: %s)", username, request.remote_addr
        )
        if throttled:
            auth_limiter.record_failure(request.remote_addr, username)
        return jsonify({"error": "invalid credentials"}), 401
    # A success clears the per-account counter only: letting it also clear the
    # per-IP one handed any attacker who owns one valid account a free reset
    # between brute-force bursts.
    auth_limiter.reset_user(username)
    # Never keep anything the pre-auth session carried (session fixation), and
    # stamp the epoch so a later password/role change revokes this cookie.
    session.clear()
    session["uid"] = str(user.id)
    session["epoch"] = user.session_epoch or 0
    session.permanent = True
    from .accessapi import me_access
    from .sourcesapi import sources_brief

    return jsonify(
        {
            "user": {
                "id": _uuid_text(user.id),
                "name": user.name,
                "admin": user.admin,
                "onboarding": user.onboarding_step or "done",
                "access": me_access(user),
                "sources": sources_brief(),
            }
        }
    )


@webapi.route("/logout", methods=["POST"])
def logout():
    # A remote-control session is somebody else's account lent to this browser:
    # logging out of it ends the loan and hands the browser back its own, rather
    # than logging its owner out of an account they never lent.
    if session.get("rc"):
        from .remote import _end_remote_session

        return jsonify({"ok": True, "restored": _end_remote_session()})
    session.clear()
    return jsonify({"ok": True})


@webapi.route("/me")
@login_required
def me():
    u = request.webuser
    from .accessapi import me_access
    from .sourcesapi import sources_brief

    # What this person may use of each provider (plan multi-provider §15), so
    # the web app offers nothing that would be refused; and the sources in
    # their order, so that no screen writes a source's name itself.
    out = {
        "user": {
            "id": _uuid_text(u.id),
            "name": u.name,
            "admin": u.admin,
            "onboarding": u.onboarding_step or "done",
            "access": me_access(u),
            "sources": sources_brief(),
        }
    }
    # A remote-control session is logged in as the owner, with a narrower
    # grant; the web app needs both to know whose player it is driving.
    link = getattr(request, "rc", None)
    if link is not None:
        from .remote import describe

        out["remote"] = describe(link)
    return jsonify(out)


# -- discovery --------------------------------------------------------------


def _tracks_in_order(ids):
    """The API dicts of the tracks ``ids`` (row UUIDs), in that order."""
    ids = list(ids)
    if not ids:
        return []
    key = {
        r[0]: str(r[1]) if r[1] is not None else str(r[0])
        for r in Track.select(Track.id, Track.deezer_id).where(Track.id.in_(ids)).tuples()
    }
    rows = _db_tracks(
        Track.select(Track, Album, Artist)
        .join(Album)
        .switch(Track)
        .join(Artist)
        .where(Track.id.in_(ids))
    )
    by_key = {r["deezer_id"]: r for r in rows}
    return [by_key[key[i]] for i in ids if i in key and key[i] in by_key]


def _data(fn):
    """Call a public-API method and return its ``data`` list, never raising."""
    try:
        return (fn() or {}).get("data", []) or []
    except Exception:
        _log_deezer_failure("Deezer public API call failed")
        return []


def _side_by_side(*fns):
    """Run independent Deezer calls at once; ``[(value, exc), ...]`` in order.

    A screen that needs several calls paid their SUM one after the other — a
    search was five public-API round trips, an artist four, an album or a
    playlist two — when nothing in one depends on another. Network only: the
    database stays on the request's own thread. Every call still goes through
    the shared limiter and the circuit breaker, so this asks Deezer nothing
    more, only sooner.
    """

    def call(fn):
        try:
            return fn(), None
        except Exception as exc:  # handed back to the caller, which decides
            return None, exc

    if len(fns) == 1:
        return [call(fns[0])]
    with ThreadPoolExecutor(max_workers=len(fns)) as pool:
        return [f.result() for f in [pool.submit(call, fn) for fn in fns]]

PUBLIC_PAGE = 1000
PUBLIC_MAX = 10000


def _public_all(fetch):
    """Every item of a paged public-API list (``fetch(index, limit)``), page
    after page until the API says there is no next one. Raises what the call
    raises, so the caller can tell an outage from an empty list."""
    items, index = [], 0
    while len(items) < PUBLIC_MAX:
        page = fetch(index, PUBLIC_PAGE) or {}
        batch = page.get("data") or []
        items.extend(batch)
        if not batch or not page.get("next"):
            break
        index += len(batch)
    return items


def _public_tracks(raw, album=None):
    """Public-API tracks as the pages serve them. An album's tracklist leaves
    the album out of each track (it is the page's), so it is put back."""
    out = []
    for t in raw or []:
        if album is not None and isinstance(t, dict) and not t.get("album"):
            t = dict(t, album=album)
        x = _track_api(t)
        if x:
            out.append(x)
    return flag_unavailable(out)


def _public_album_page(album_id):
    """The album page from the public API alone (lot P3: nobody lends this
    person an account): the album and its full tracklist, side by side."""
    api = _dz_api()
    if api is None:
        return _deezer_missing()
    (info, exc), (songs, songs_exc) = _side_by_side(
        lambda: api.get_album(album_id),
        lambda: _public_all(lambda i, n: api.get_album_tracks(album_id, index=i, limit=n)),
    )
    if exc is not None:
        _log_deezer_failure("album %s public page failed", album_id, exc=exc)
        return _deezer_missing(exc)
    if not (info or {}).get("id"):
        return jsonify({"error": "not found"}), 404
    if songs_exc is not None:
        songs = ((info.get("tracks") or {}).get("data")) or []
    out = _album_api(info)
    out["cover"] = _pic(info, "cover_big", "cover_medium", "cover") or out["cover"]
    out["is_favorite"] = _starred("album", deezer_id=album_id)
    return jsonify({"album": out, "tracks": _public_tracks(songs, album=info)})


def _public_playlist_page(playlist_id):
    """A Deezer playlist's page from the public API alone (lot P3)."""
    api = _dz_api()
    if api is None:
        return _deezer_missing()
    (info, exc), (songs, songs_exc) = _side_by_side(
        lambda: api.get_playlist(playlist_id),
        lambda: _public_all(lambda i, n: api.get_playlist_tracks(playlist_id, index=i, limit=n)),
    )
    if exc is not None:
        _log_deezer_failure("playlist %s public page failed", playlist_id, exc=exc)
        return _deezer_missing(exc)
    if not (info or {}).get("id"):
        return jsonify({"error": "not found"}), 404
    if songs_exc is not None:
        songs = ((info.get("tracks") or {}).get("data")) or []
    out = _playlist_api(info)
    out["duration"] = int(info.get("duration") or 0)
    out["editable"] = False
    return jsonify({"playlist": out, "tracks": _public_tracks(songs)})


def _public_discography(artist_id):
    """An artist's releases from the public API alone (lot P3), in the tabs
    the gateway's discography uses: ``all`` and one per record type."""
    api = _dz_api()
    if api is None:
        return {}
    try:
        raw = _public_all(lambda i, n: api.get_artist_albums(artist_id, index=i, limit=n))
    except Exception:
        _log_deezer_failure("discography %s public failed", artist_id)
        return {}
    out = {}
    for r in raw:
        if not isinstance(r, dict) or not r.get("id"):
            continue
        kind = r.get("record_type") or "album"
        item = {
            "deezer_id": str(r.get("id")),
            "title": r.get("title", ""),
            "cover": _pic(r, "cover_medium", "cover_big", "cover"),
            "release_date": (str(r.get("release_date") or "")[:10]) or None,
            "year": (str(r.get("release_date") or "")[:4]) or None,
            "record_type": kind,
            "nb_tracks": r.get("nb_tracks"),
        }
        out.setdefault("all", []).append(item)
        out.setdefault(kind, []).append(item)
    for items in out.values():
        items.sort(key=lambda x: x["release_date"] or "", reverse=True)
    return out


def _podcast_card(p):
    """A public-API podcast (show) search result, shaped as a card."""
    if not p or not p.get("id"):
        return None
    return {
        "deezer_id": str(p.get("id")),
        "title": p.get("title", "") or "",
        "description": p.get("description", "") or "",
        "cover": p.get("picture_xl")
        or p.get("picture_big")
        or p.get("picture_medium")
        or p.get("picture"),
        "fans": int(p.get("nb_fan") or p.get("fans") or 0),
    }


@webapi.route("/search")
@login_required
def search():
    query = request.args.get("q", "").strip()
    empty = {"artists": [], "albums": [], "tracks": [], "playlists": [], "podcasts": []}
    if not query:
        return jsonify(empty)
    try:
        limit = int(request.args.get("limit", 25))
    except (TypeError, ValueError):
        limit = 25
    limit = max(1, min(limit, 50))

    def local():
        # Local files — always available, even when Deezer is unreachable.
        try:
            return _local_search_tracks(query, limit)
        except Exception:
            logger.warning("Local search failed", exc_info=True)
            return []

    # Public API (api.deezer.com): stable, typed, returns playlists + image URLs.
    # None when Deezer is off or unreachable — the local hits still go out.
    dzapi = _dz_api()
    if dzapi is None:
        try:
            artists, albums = _local_search_entities(query, limit)
        except Exception:
            logger.warning("Local search failed", exc_info=True)
            artists, albums = [], []
        return jsonify({**empty, "tracks": local(), "artists": artists, "albums": albums})
    # The five searches run side by side (they were five round trips one after
    # the other), and the database is read on this thread while they are out.
    calls = (
        lambda: dzapi.search(query, limit=limit),
        lambda: dzapi.search_album(query, limit=limit),
        lambda: dzapi.search_artist(query, limit=limit),
        lambda: dzapi.search_playlist(query, limit=limit),
        lambda: dzapi.search_podcast(query, limit=limit),
    )
    with ThreadPoolExecutor(max_workers=len(calls)) as pool:
        futures = [pool.submit(_data, fn) for fn in calls]
        local_tracks = local()
        raw_tracks, raw_albums, raw_artists, raw_playlists, raw_podcasts = (
            f.result() for f in futures
        )
    tracks = [_track_api(t) for t in raw_tracks]
    albums = [_album_api(a) for a in raw_albums]
    artists = [_artist_api(a) for a in raw_artists]
    playlists = [_playlist_api(p) for p in raw_playlists]
    podcasts = [_podcast_card(p) for p in raw_podcasts]
    try:
        _attach_local_ids(Album, albums)
        _attach_local_ids(Artist, artists)
        _attach_local_ids(Playlist, playlists)
    except Exception:  # a shortcut is never worth failing the search over
        logger.debug("Local id lookup failed", exc_info=True)
    # Downloaded/local tracks lead (they play from disk); drop the live-search
    # duplicates of the same Deezer ids so a track isn't listed twice.
    have = {t["deezer_id"] for t in local_tracks}
    live = flag_unavailable([x for x in tracks if x and x["deezer_id"] not in have])
    return jsonify(
        {
            "tracks": local_tracks + live,
            "albums": [x for x in albums if x],
            "artists": [x for x in artists if x],
            "playlists": [x for x in playlists if x],
            "podcasts": [x for x in podcasts if x],
        }
    )


@webapi.route("/search/podcasts")
@login_required
def search_podcasts():
    query = request.args.get("q", "").strip()
    dzapi = _dz_api()
    if not query or dzapi is None:
        return jsonify({"podcasts": []})
    try:
        limit = max(1, min(int(request.args.get("limit", 25)), 50))
    except (TypeError, ValueError):
        limit = 25
    res = [
        _podcast_card(p)
        for p in _data(lambda: dzapi.search_podcast(query, limit=limit))
    ]
    return jsonify({"podcasts": [x for x in res if x]})


def _attach_local_ids(model, items):
    """Give the catalogue results the library already holds the ``id`` of their
    row (the SPA opens a page by ``id`` when there is one, so a search hit and
    the same entity reached from the library are one page, one cache entry).
    One statement; a result the library does not hold keeps only its provider
    id, which is what the catalogue page is asked by."""
    wanted = {str(x["deezer_id"]) for x in items if x and x.get("deezer_id")}
    have = {}
    wanted = list(wanted)
    for i in range(0, len(wanted), _IN_CHUNK):
        for rid, did in (
            model.select(_raw(model.id), model.deezer_id)
            .where(model.deezer_id << wanted[i : i + _IN_CHUNK])
            .tuples()
        ):
            have[str(did)] = _uuid_text(rid)
    for x in items:
        if x and str(x.get("deezer_id")) in have:
            x["id"] = have[str(x["deezer_id"])]
    return items


def _entity_row(model, make_uuid, raw):
    """The library row of an album / artist named by its UUID or a Deezer id,
    or None. Database only; a private upload nobody here may read is no row."""
    text = str(raw)
    try:
        key = uuid.UUID(text)
        row = model.get_or_none(model.id == key)
    except ValueError:
        if not _valid_id(text):
            return None
        row = model.select().where(
            (model.deezer_id == text) | (model.id == make_uuid(text))
        ).first()
    if row is None or row.deezer_id is not None:
        return row
    # No Deezer id: an upload or a scanned folder, shown only with a track the
    # caller may read.
    field = Track.album if model is Album else Track.artist
    seen = _visible_tracks(Track.select(Track.id).where(field == row)).limit(1)
    return row if seen.exists() else None


def _artist_row(raw):
    from ..deezer import ids as dz_ids

    return _entity_row(Artist, dz_ids.artist_uuid, raw)


def _album_row(raw):
    from ..deezer import ids as dz_ids

    return _entity_row(Album, dz_ids.album_uuid, raw)


@webapi.route("/artist/<artist_id>")
@login_required
def artist(artist_id):
    """The artist page. A library artist is answered from the database alone;
    Deezer is only the page of an artist we do not hold (its facts: the
    artist and the albums, no shelves of recommendations), and the part of a
    library artist that is only on Deezer is ``/artist/<id>/on-deezer``."""
    ar = _artist_row(artist_id)
    if ar is not None:
        return jsonify(_db_artist_response(ar))
    # The id is concatenated into the outgoing api.deezer.com URL: validate it
    # here rather than letting "%3F..." smuggle query parameters into the call.
    dzapi = _dz_api() if _valid_id(artist_id) else None
    exc = None
    if dzapi is not None:
        (info, exc), (albums, _) = _side_by_side(
            lambda: dzapi.get_artist(artist_id),
            lambda: _data(lambda: dzapi.get_artist_albums(artist_id, limit=50)),
        )
        if exc is not None:
            _log_deezer_failure("artist %s lookup failed", artist_id, exc=exc)
        if info and info.get("id"):
            artist_out = _artist_api(info)
            artist_out["is_favorite"] = _starred("artist", deezer_id=artist_id)
            return jsonify(
                {
                    "artist": artist_out,
                    "bio": None,
                    "top": [],
                    "albums": [x for x in map(_album_api, albums or []) if x],
                    "related": [],
                }
            )
    if not _valid_id(artist_id):
        return jsonify({"error": "not found"}), 404
    return _deezer_missing(exc)


@webapi.route("/artist/<artist_id>/on-deezer")
@login_required
def artist_on_deezer(artist_id):
    """The albums of an artist that Deezer has and the library does not: the
    part of the page that needs Deezer, asked after the page is on screen."""
    ar = _artist_row(artist_id)
    did = str(ar.deezer_id) if ar is not None and ar.deezer_id else str(artist_id)
    if not _valid_id(did):
        return jsonify({"albums": []})
    if _provider() is None and not _deezer_anonymous():
        # switched off (or not for this person): nothing more to offer, no error
        return jsonify({"albums": []})
    dzapi = _dz_api()
    if dzapi is None:
        return _deezer_missing()
    try:
        raw = (dzapi.get_artist_albums(did, limit=50) or {}).get("data") or []
    except Exception as exc:
        _log_deezer_failure("artist %s albums failed", did, exc=exc)
        return _deezer_missing(exc)
    albums = [x for x in map(_album_api, raw or []) if x]
    return jsonify({"albums": _not_in_library(Album, albums)})


def _not_in_library(model, items):
    """`items` (API dicts carrying a ``deezer_id``) whose row the library does
    not hold."""
    ids_ = [str(x.get("deezer_id")) for x in items if x.get("deezer_id")]
    have = set()
    for i in range(0, len(ids_), _IN_CHUNK):
        have.update(
            str(d)
            for (d,) in model.select(model.deezer_id)
            .where(model.deezer_id << ids_[i : i + _IN_CHUNK])
            .tuples()
        )
    return [x for x in items if str(x.get("deezer_id")) not in have]


@webapi.route("/artist/<artist_id>/tracks")
@login_required
def artist_tracks(artist_id):
    """All of an artist's tracks as a playlist-like list (the 'Titres' tab).

    A library artist: the tracks the library holds. Otherwise the public API's
    /artist/{id}/top (paged, so several pages gathered, deduped, up to a cap).
    """
    PAGE = 100
    MAX = 300
    ar = _artist_row(artist_id)
    if ar is not None:
        rows = _visible_tracks(
            Track.select(Track, Album, Artist)
            .join(Album)
            .switch(Track)
            .join(Artist)
            .where(Track.artist == ar)
        ).order_by(Album.name, Track.disc, Track.number)
        return jsonify({"tracks": _db_tracks(rows)})
    dzapi = _dz_api() if _valid_id(artist_id) else None
    failure = None
    if dzapi is not None:
        tracks = []
        seen = set()
        index = 0
        while len(tracks) < MAX:
            try:
                batch = (dzapi.get_artist_top(artist_id, index=index, limit=PAGE) or {}).get("data") or []
            except Exception as exc:
                _log_deezer_failure("artist %s tracks failed", artist_id)
                failure = exc
                break
            if not batch:
                break
            for raw in batch:
                t = _track_api(raw)
                if t and t["deezer_id"] not in seen:
                    seen.add(t["deezer_id"])
                    tracks.append(t)
            if len(batch) < PAGE:
                break
            index += PAGE
        if tracks:
            return jsonify({"tracks": tracks})
    if not _valid_id(artist_id):
        return jsonify({"error": "not found"}), 404
    return _deezer_missing(failure)


@webapi.route("/album/<album_id>")
@login_required
def album(album_id):
    """The album page. A library album is answered from the database alone
    (the tracks the library holds); the rest of its tracklist is
    ``/album/<id>/on-deezer``. Deezer is the page only of an album we do not
    hold."""
    alb = _album_row(album_id)
    if alb is not None:
        return jsonify(_db_album_response(alb))
    provider = _provider()
    exc = None
    if provider is None and _deezer_anonymous() and _valid_id(album_id):
        return _public_album_page(album_id)
    if provider is not None and _valid_id(album_id):
        # The page (header + favourite state) and the FULL tracklist side by
        # side: the page alone only carries the first tracks.
        (page, exc), (songs, songs_exc) = _side_by_side(
            lambda: provider.dz.gw.get_album_page(album_id),
            lambda: provider.get_album_tracks(album_id),
        )
        if exc is not None:
            _log_deezer_failure("album %s page failed", album_id, exc=exc)
        else:
            data = (page or {}).get("DATA") or {}
            if data.get("ALB_ID"):
                if songs_exc is not None:
                    songs = ((page or {}).get("SONGS") or {}).get("data", [])
                alb_out = _album(data)
                alb_out["is_favorite"] = _starred("album", deezer_id=album_id)
                return jsonify({"album": alb_out, "tracks": _tracks(songs)})
    if not _valid_id(album_id):
        return jsonify({"error": "not found"}), 404
    return _deezer_missing(exc)


@webapi.route("/album/<album_id>/on-deezer")
@login_required
def album_on_deezer(album_id):
    """The tracks of an album that Deezer has and the library does not."""
    alb = _album_row(album_id)
    did = str(alb.deezer_id) if alb is not None and alb.deezer_id else str(album_id)
    provider = _provider()
    if not _valid_id(did):
        return jsonify({"tracks": []})
    if provider is None and _deezer_anonymous():
        api = _dz_api()
        if api is None:
            return _deezer_missing()
        try:
            raw = _public_all(lambda i, n: api.get_album_tracks(did, index=i, limit=n))
        except Exception as exc:
            _log_deezer_failure("album %s public tracks failed", did, exc=exc)
            return _deezer_missing(exc)
        album = {"id": did, "title": alb.name if alb is not None else ""}
        return jsonify({"tracks": _not_in_library(Track, _public_tracks(raw, album=album))})
    if provider is None:  # switched off: nothing more to offer, no error
        return jsonify({"tracks": []})
    if not provider.available():
        return _deezer_missing()
    try:
        songs = provider.get_album_tracks(did)
    except Exception as exc:
        _log_deezer_failure("album %s tracks failed", did, exc=exc)
        return _deezer_missing(exc)
    tracks = [t for t in _tracks(songs) if t]
    return jsonify({"tracks": _not_in_library(Track, tracks)})


def _is_uuid_text(value) -> bool:
    """A string that parses as a UUID — never a Deezer numeric/slug id."""
    try:
        uuid.UUID(str(value))
    except (ValueError, TypeError, AttributeError):
        return False
    return True


@webapi.route("/playlist/<playlist_id>")
@login_required
def playlist(playlist_id):
    # A user playlist lives in the DB (editable, may contain local files); a
    # recommendation/editorial playlist is read straight from Deezer (read-only).
    pl = _resolve_db_playlist(playlist_id)
    if pl is not None:
        user = request.webuser
        role = placcess.role_for(pl, user)
        # A non-member sees the SAME 404 as an unknown id: the answer must not
        # confirm that a real playlist exists (plan §5, §12).
        if role is None:
            return jsonify({"error": "not found"}), 404
        # The caller's VISIBLE rows are the list, the count and the cover: a
        # shared playlist never becomes a side channel for a private upload.
        tracks = _db_tracks(_db_playlist_track_rows(pl, user))
        is_admin = bool(user.admin)
        editable = is_admin or placcess.role_can_edit(role)
        manage = is_admin or placcess.role_can_manage(role)
        # Members first: their query carries the owner's name, so a
        # collaborative read does not pay a separate `playlist.user` lookup.
        # A public reader carries no member names and reads the owner alone.
        members = placcess.member_rows(pl) if role != "public" else None
        out = {
            "id": str(pl.id),
            "deezer_id": pl.deezer_id,
            "title": pl.name,
            "description": pl.comment or "",
            "cover": _db_playlist_cover(tracks),
            "nb_tracks": len(tracks),
            "owner": members[0]["name"] if members else pl.user.name,
            "is_favorite": _starred("playlist", row=pl),
            "role": role,
            "admin": is_admin,
            "editable": editable,
            "can_manage": manage,
            "version": pl.version,
            "members_version": pl.members_version,
        }
        if members is not None:
            # A public link carries no member names and no author hint: the
            # owner name above is the one the page already shows.
            out["last_edited_by"] = pl.last_edited_by
            out["last_edited_at"] = (
                pl.last_edited_at.isoformat() if pl.last_edited_at else None
            )
            out["members"] = members
        if manage:
            out["private_tracks"] = _playlist_private_tracks(pl)
        return jsonify({"playlist": out, "tracks": tracks})

    # No local row. A UUID names something this server should own, so it is a
    # 404 rather than a Deezer page; a numeric id keeps the editorial page.
    if _is_uuid_text(playlist_id):
        return jsonify({"error": "not found"}), 404

    if _provider() is None and _deezer_anonymous() and _valid_id(playlist_id):
        return _public_playlist_page(playlist_id)
    provider, err = _need_provider()
    if err:
        return err
    # pagePlaylist only returns the first page of songs, so the full list is
    # asked for too — at the same time, not after it.
    (page, exc), (songs, songs_exc) = _side_by_side(
        lambda: provider.get_playlist_page(playlist_id),
        lambda: provider.get_playlist_tracks(playlist_id),
    )
    if exc is not None:
        _log_deezer_failure("playlist %s page failed", playlist_id, exc=exc)
        return _deezer_missing(exc)
    page = page or {}
    data = page.get("DATA") or {}
    if not data.get("PLAYLIST_ID"):
        return jsonify({"error": "not found"}), 404
    if songs_exc is not None:
        songs = (page.get("SONGS") or {}).get("data", [])
    out = _playlist(data)
    out["editable"] = False
    return jsonify({"playlist": out, "tracks": _tracks(songs)})


@webapi.route("/artist/<artist_id>/discography")
@login_required
def discography(artist_id):
    ar = _artist_row(artist_id)
    if ar is not None:
        # The library's own albums; the others are ``/artist/<id>/on-deezer``.
        albums = Album.select(Album, Artist).join(Artist).where(Album.artist == ar)
        cards = _db_album_cards(albums)
        cards.sort(key=lambda x: str(x.get("year") or ""), reverse=True)
        # Keys the Artist page reads: `album` (grid) and `all` (latest shelf).
        return jsonify({"discography": {"album": cards, "all": cards} if cards else {}})
    if not _valid_id(artist_id):
        return jsonify({"error": "not found"}), 404
    provider = _provider()
    tabs = {}
    if provider is None and _deezer_anonymous():
        return jsonify({"discography": _public_discography(artist_id)})
    if provider is not None:
        try:
            tabs = provider.get_artist_discography(artist_id)
        except Exception:
            _log_deezer_failure("discography %s failed", artist_id)
            tabs = {}
    out = {}
    for tab, releases in (tabs or {}).items():
        items = [
            {
                "deezer_id": str(r.get("id")),
                "title": r.get("title", ""),
                "cover": _image("cover", r.get("md5_image")),
                "release_date": (str(r.get("release_date") or "")[:10]) or None,
                "year": (str(r.get("release_date") or "")[:4]) or None,
                "record_type": r.get("record_type"),
                "nb_tracks": r.get("nb_song"),
            }
            for r in (releases or [])
        ]
        # Surface the most recent releases first so the client can show a
        # "latest releases" shelf without re-sorting (the gw order isn't reliable).
        items.sort(key=lambda x: x["release_date"] or "", reverse=True)
        out[tab] = items
    return jsonify({"discography": out})


def _lyrics_in_background(track_id):
    """Ask LRCLIB for an archived track's lyrics once, after the response."""
    app = current_app._get_current_object()

    def run():
        from ..db import close_connection, open_connection
        from ..deezer import lyrics as deezer_lyrics

        with app.app_context():
            try:
                open_connection(reuse=True)
                track = Track.get_or_none(Track.id == track_id)
                if track is not None:
                    deezer_lyrics.ensure_lyrics(None, track)
            except Exception:
                logger.debug("background lyrics failed", exc_info=True)
            finally:
                close_connection()

    threading.Thread(target=run, name="lyrics-bg", daemon=True).start()


_LYRICS_COLD_TIMEOUT = (2, 4)


@webapi.route("/lyrics/<track_id>")
@login_required
def lyrics(track_id):
    from ..deezer import lyrics as deezer_lyrics

    key = _gain_key(track_id)
    track = Track.get_or_none(Track.id == key) if key is not None else None
    if track is None or not _may_access_track(track):
        return jsonify({"lyrics": None})

    # Archived: the .lrc sidecar, read from disk. A track LRCLIB was never asked
    # about is searched AFTER the answer, never in the request.
    if track.path and os.path.isfile(track.path):
        lyr = deezer_lyrics.read_sidecar(track)
        if lyr is None or not lyr.get("synced"):
            if not deezer_lyrics.lyrics_tried(track):
                _lyrics_in_background(track.id)
        return jsonify({"lyrics": lyr})

    # Not archived (a row with no file): one short LRCLIB request, its answer
    # kept in memory for the archive to write (deezer/lyrics.remember_cold).
    artist = track.artist.name if track.artist else None
    album = track.album.name if track.album else None
    lyr = deezer_lyrics.fetch_cold(track, artist, album, timeout=_LYRICS_COLD_TIMEOUT)
    return jsonify({"lyrics": lyr})


def _gain_key(raw):
    """The row id a gain request names: a UUID as is, a Deezer numeric id
    through ``ids.track_uuid`` (a local computation); None for anything else."""
    import uuid as _uuid

    from ..deezer import ids

    text = str(raw or "")
    try:
        return _uuid.UUID(text)
    except ValueError:
        pass
    return ids.track_uuid(text) if _valid_id(text) else None


def _gain_visible(row):
    """Whether this row's gain may be shown to the caller (uploads are private)."""
    owner = getattr(row, "owner_id", None)
    if owner is None:
        return True
    user = getattr(request, "webuser", None)
    return bool(user and (user.admin or user.id == owner))


@webapi.route("/gain/<track_id>")
@login_required
def track_gain(track_id):
    # ReplayGain for the web player's volume normalization: the loudness measured
    # on the file (analysis), stored on the row. Database only — no Deezer call.
    key = _gain_key(track_id)
    if key is None:
        return jsonify({"gain": None})
    row = Track.select(Track.gain, Track.owner).where(Track.id == key).first()
    if row is None or not _gain_visible(row):
        return jsonify({"gain": None})
    return jsonify({"gain": row.gain})


# How many ids one batch may ask about. The player asks for what it is about to
# play plus its lookahead window — a handful — so this is generous.
GAIN_BATCH_MAX = 100


@webapi.route("/gains", methods=["POST"])
@login_required
def track_gains():
    """ReplayGain for a whole run of tracks in ONE query.

    The player normalizes a track at the instant its source is swapped in, so a
    gain that arrives later is a volume change in the middle of a song. This is
    what lets it ask for everything it is about to play before any of it starts.
    Ids may be UUIDs or Deezer numeric ids; the answer is keyed by the id as
    asked. The database is the only source: an unknown gain stays null (that
    track simply isn't normalized).
    """
    data = request.get_json(silent=True) or {}
    raw = data.get("ids") or []
    if not isinstance(raw, list):
        return jsonify({"error": "ids must be a list"}), 400
    asked = list(dict.fromkeys(str(x) for x in raw))
    keys = {}
    for i in asked:
        k = _gain_key(i)
        if k is not None:
            keys[i] = k
        if len(keys) >= GAIN_BATCH_MAX:
            break
    gains = {i: None for i in keys}
    if keys:
        wanted = set(keys.values())
        try:
            found = {
                r.id: r
                for r in Track.select(Track.id, Track.gain, Track.owner).where(
                    Track.id.in_(list(wanted))
                )
            }
        except Exception:
            logger.warning("gain batch DB lookup failed", exc_info=True)
            found = {}
        for i, k in keys.items():
            r = found.get(k)
            if r is not None and _gain_visible(r):
                gains[i] = r.gain
    return jsonify({"gains": gains})


# -- radio / flow / recommendations -----------------------------------------


_RADIO_SIZE = 40


@webapi.route("/radio/track/<track_id>")
@login_required
def track_radio(track_id):
    """A radio seeded from one track (neezer/radio.py): anchored on it,
    drawn afresh on every call, at most three titles of one artist."""
    seed = refs.resolve_track(track_id)
    if seed is None:
        return jsonify({"tracks": []})
    ids = radio.build(request.webuser, [seed.id], _RADIO_SIZE)
    return jsonify({"tracks": _tracks_in_order(ids)})


@webapi.route("/radio/artist/<artist_id>")
@login_required
def artist_radio(artist_id):
    """Artist radio: the artist's own best titles among what sounds like them,
    the artist first."""
    ar = _artist_row(artist_id)
    if ar is None:
        return jsonify({"tracks": []})
    seeds, own = radio.artist_seeds(request.webuser, ar.id)
    if not seeds:
        return jsonify({"tracks": []})
    ids = seeds[:1] + radio.build(
        request.webuser, seeds, _RADIO_SIZE - 1, exclude=seeds[:1], seed_artist=ar.id, own=own,
        block_seeds=False,
    )
    return jsonify({"tracks": _tracks_in_order(ids)})


def _ref_list(data, name, limit):
    raw = data.get(name) or []
    if not isinstance(raw, list):
        return []
    raw = [str(x)[:64] for x in raw[-limit:] if isinstance(x, (str, int))]
    return [t.id for t in refs.resolve_tracks(raw) if t is not None]


@webapi.route("/radio/next", methods=["POST"])
@login_required
def radio_next():
    """The next titles of a radio already playing: the ORIGINAL seed (a track
    or an artist) plus what this session heard through, skipped and queued,
    so the radio follows the listener without drifting off its seed."""
    data = request.get_json(silent=True) or {}
    seed = data.get("seed") or {}
    if not isinstance(seed, dict):
        return jsonify({"error": "invalid seed"}), 400
    kind = seed.get("kind")
    sid = str(seed.get("id") or "")[:64]
    try:
        n = max(1, min(int(data.get("n") or _RADIO_SIZE), _RADIO_SIZE))
    except (TypeError, ValueError):
        n = _RADIO_SIZE
    recent = _ref_list(data, "recent", 20)
    skipped = _ref_list(data, "skipped", 30)
    exclude = _ref_list(data, "exclude", 400)
    user = request.webuser
    if kind == "artist":
        ar = _artist_row(sid)
        if ar is None:
            return jsonify({"tracks": []})
        seeds, own = radio.artist_seeds(user, ar.id)
        ids = radio.build(user, seeds, n, recent, skipped, exclude, seed_artist=ar.id, own=own,
                          block_seeds=False)
    elif kind == "track":
        track = refs.resolve_track(sid)
        # A catalogue title the library does not hold yet (a new release, a
        # search result: its row is made by its first play, which may still
        # be under way) radios on its artist, which the caller names.
        ar = _artist_row(str(seed.get("artist") or "")[:64]) if track is None and seed.get("artist") else None
        if track is not None:
            ids = radio.build(user, [track.id], n, recent, skipped, exclude)
        elif ar is not None:
            seeds, own = radio.artist_seeds(user, ar.id)
            ids = radio.build(user, seeds, n, recent, skipped, exclude, seed_artist=ar.id, own=own,
                              block_seeds=False)
        else:
            return jsonify({"tracks": []})
    else:
        return jsonify({"error": "invalid seed"}), 400
    return jsonify({"tracks": _tracks_in_order(ids)})


# -- my library -------------------------------------------------------------


@webapi.route("/me/playlists")
@login_required
def my_playlists():
    """The caller's own playlists plus the ones shared with them.

    NEVER an admin firehose: an admin sees their own lists and their
    memberships, exactly like everybody else; they can still open any playlist
    by URL. A public playlist is not listed here (its Subsonic meaning is
    unchanged, but the web sidebar/Library are personal). Folders are the
    person's own organisation of the same list, in the same answer and the same
    statement budget (6 at 61 playlists, 8 at 500 — two chunked pairs double at
    ``_IN_CHUNK``), and every count/cover uses the caller's VISIBLE rows.
    """
    from .playlist_folders import list_shape

    user = request.webuser
    member = PlaylistMember.alias("member")
    owner = User.alias("owner")
    query = (
        Playlist.select(
            _raw(Playlist.id),
            Playlist.deezer_id,
            Playlist.name,
            # `created` and the caller's placement join the same SELECT: the
            # rendering order is decided in Python (playlist_folders.list_shape)
            # and costs no extra statement. A playlist with no placement row is
            # unfiled and sorts by creation date, which is today's list.
            Playlist.created,
            _raw(PlaylistPlacement.folder),
            PlaylistPlacement.position,
            # The role is decided in the same SELECT: `user_id` first (an owner
            # may have no member row), then the caller's membership row.
            _raw(Playlist.user),
            owner.name,
            member.role,
        )
        .join(owner, on=(Playlist.user == owner.id))
        .left_outer_join(
            member,
            on=((member.playlist == Playlist.id) & (member.user == user.id)),
        )
        .left_outer_join(
            PlaylistPlacement,
            on=(
                (PlaylistPlacement.playlist == Playlist.id)
                & (PlaylistPlacement.user == user.id)
            ),
        )
        .where((Playlist.user == user) | (member.user == user.id))
        .tuples()
    )
    rows = list(query)
    lists = [row[:6] for row in rows]
    roles = {}
    for row in rows:
        pid = row[0]
        mine = _uuid_text(row[6]) == str(user.id)
        stored = row[8] if row[8] in placcess.ROW_ROLES else None
        roles[pid] = (
            "owner" if mine else (stored or "viewer"),
            row[7],
        )
    folders, entries = list_shape(user, lists)
    # Count + first-track cover only — loading every track of every playlist
    # just for this made the list (refetched after each playlist edit) scale
    # with the whole library. And both in ONE query each for all the
    # playlists: asked per playlist, it was two statements a playlist (122 for
    # sixty, measured, 90 ms). The visibility filter is the caller's: a viewer's
    # count and cover are exactly what their playlist page shows.
    visible = Track.visible_clause(user)
    counts = {}
    covers = {}
    pids = [p[0] for p in lists]
    for i in range(0, len(pids), _IN_CHUNK):
        chunk = pids[i : i + _IN_CHUNK]
        count_query = (
            PlaylistTrack.select(_raw(PlaylistTrack.playlist), fn.COUNT(PlaylistTrack.id))
            .join(Track, on=(PlaylistTrack.track == Track.id))
            .where(PlaylistTrack.playlist << chunk)
            .group_by(PlaylistTrack.playlist)
        )
        if visible is not None:
            count_query = count_query.where(visible)
        for pid, nb in count_query.tuples():
            counts[pid] = nb
        first = (
            PlaylistTrack.select(
                PlaylistTrack.playlist.alias("pl"), fn.MIN(PlaylistTrack.index).alias("ix")
            )
            .join(Track)
            .join(Album)
            .switch(Track)
            .join(Artist)
            .where(PlaylistTrack.playlist << chunk)
        )
        if visible is not None:
            first = first.where(visible)
        first = first.group_by(PlaylistTrack.playlist).alias("first")
        for pid, alid in (
            PlaylistTrack.select(_raw(PlaylistTrack.playlist), _raw(Album.id))
            .join(first, on=((PlaylistTrack.playlist == first.c.pl) & (PlaylistTrack.index == first.c.ix)))
            .switch(PlaylistTrack)
            .join(Track)
            .join(Album)
            .tuples()
        ):
            if pid in covers:
                continue
            covers[pid] = cover_url_for(alid)
    # The folder counts, one grouped statement for all of them (never one per
    # folder), and only for playlists this person may see — an inert placement
    # left in the database must not inflate a badge. A membership is visible
    # exactly like an owned list.
    membership = PlaylistMember.select(PlaylistMember.playlist).where(
        PlaylistMember.user == user.id
    )
    counts_by_folder = {}
    for fid, nb in (
        PlaylistPlacement.select(
            _raw(PlaylistPlacement.folder), fn.COUNT(PlaylistPlacement.playlist)
        )
        .join(Playlist, on=(PlaylistPlacement.playlist == Playlist.id))
        .where(
            (PlaylistPlacement.user == user)
            & PlaylistPlacement.folder.is_null(False)
            & ((Playlist.user == user) | Playlist.id.in_(membership))
        )
        .group_by(PlaylistPlacement.folder)
        .tuples()
    ):
        counts_by_folder[_uuid_text(fid)] = nb
    for folder in folders:
        folder["nb_playlists"] = counts_by_folder.get(folder["id"], 0)
    head = {pid: (dz, name) for pid, dz, name, _created, _fid, _pos in lists}
    out = []
    for entry in entries:
        dz, name = head[entry["pid"]]
        role, owner_name = roles[entry["pid"]]
        out.append(
            {
                "id": _uuid_text(entry["pid"]),
                "deezer_id": dz,
                "title": name,
                "cover": covers.get(entry["pid"]),
                "nb_tracks": counts.get(entry["pid"], 0),
                "role": role,
                # The admin override is true either way (§5.1), exactly like
                # the playlist page: hiding is UX, the route checks again.
                "editable": bool(
                    user.admin or placcess.role_can_edit(role)
                ),
                "can_manage": bool(
                    user.admin or placcess.role_can_manage(role)
                ),
                "owner": owner_name,
                # Additive only: an older SPA and the Android car overlay ignore
                # both keys. `folder_id` is null when unfiled, `position` null
                # when the playlist has no placement row (never ordered).
                "folder_id": entry["folder_id"],
                "position": entry["position"],
            }
        )
    return jsonify({"playlists": out, "folders": folders})


@webapi.route("/me/favorite-ids")
@login_required
def my_favorite_ids():
    """The ids of everything the user starred (cheap) — for accurate heart state
    in the UI. Read from the database only: nothing asks Deezer."""
    # Two columns, as text: this used to hydrate a Track, an Album and an
    # Artist per star (twice, local then Deezer) to read one id from each —
    # 314 ms for 4000 stars, measured, for a 40 KB answer.
    rows = (
        Track.select(_raw(Track.id), Track.deezer_id)
        .join(StarredTrack, on=(StarredTrack.starred == Track.id))
        .where(StarredTrack.user == request.webuser)
        .order_by(StarredTrack.date.desc())
        .tuples()
    )
    out = {"ids": [str(dz) if dz else _uuid_text(tid) for tid, dz in rows]}
    # Albums, artists and playlists: the Deezer id when the row has one, else its
    # UUID, like the tracks.
    for key, (model, star, _) in zip(("albums", "artists", "playlists"),
                                     (_star_kinds()[k] for k in ("album", "artist", "playlist"))):
        out[key] = [
            str(dz) if dz else _uuid_text(rid)
            for rid, dz in model.select(_raw(model.id), model.deezer_id)
            .join(star, on=(star.starred == model.id))
            .where(star.user == request.webuser)
            .tuples()
        ]
    # The titles the person said they do not like, by the same key, so the
    # menus show "Je n'aime plus" without asking again.
    from ..db import TrackDislike

    out["disliked"] = [
        str(dz) if dz else _uuid_text(tid)
        for tid, dz in Track.select(_raw(Track.id), Track.deezer_id)
        .join(TrackDislike, on=(TrackDislike.track == Track.id))
        .where(TrackDislike.user == request.webuser)
        .tuples()
    ]
    return jsonify(out)


@webapi.route("/me/local")
@login_required
def my_local():
    """All local (imported/uploaded) tracks — the home of your own files.

    Podcast episodes are not files of yours: they have their own rows and
    routes, and a Track an older scan made from one is left out here (the
    audio itself is never touched)."""
    q = _visible_tracks(
        Track.select(Track, Album, Artist)
        .join(Album)
        .switch(Track)
        .join(Artist)
        .switch(Track)
        .where(Track.deezer_id.is_null(True) & Track.youtube_id.is_null(True))
    )
    q = q.where(_not_from_podcasts())
    q = q.order_by(Artist.name, Album.name, Track.disc, Track.number).limit(5000)
    return jsonify({"tracks": [_local_track(t) for t in q]})


@webapi.route("/me/favorites")
@login_required
def my_favorites():
    # The database is the library: every star, local or imported, in one query.
    # Deezer is never asked (importing the account is a separate, one-off action).
    return jsonify({"tracks": _db_tracks(_user_starred_query())})


@webapi.route("/me/artists")
@login_required
def my_artists():
    """The artists the person starred, each with what the library holds of
    them and how much the person listened to them: the "Artistes" list of the
    library and the home's row. Three statements whatever the count."""
    from ..db import ListenEvent, StarredArtist

    user = request.webuser
    stars = list(
        Artist.select(_raw(Artist.id), Artist.name, Artist.deezer_id, StarredArtist.date)
        .join(StarredArtist, on=(StarredArtist.starred == Artist.id))
        .where(StarredArtist.user == user)
        .tuples()
    )
    ids = [r[0] for r in stars]
    counts, plays = {}, {}
    if ids:
        id_col = _raw(Track.artist)
        for i in range(0, len(ids), _IN_CHUNK):
            chunk = [uuid.UUID(x) for x in ids[i : i + _IN_CHUNK]]
            for aid, n in (
                _visible_tracks(Track.select(id_col, fn.COUNT(Track.id)).where(Track.artist.in_(chunk)))
                .group_by(Track.artist)
                .tuples()
            ):
                counts[_uuid_text(aid)] = n
            for aid, n in (
                ListenEvent.select(id_col, fn.COUNT(ListenEvent.id))
                .join(Track, on=(ListenEvent.track == Track.id))
                .where((ListenEvent.user == user) & (ListenEvent.outcome > 0) & Track.artist.in_(chunk))
                .group_by(Track.artist)
                .tuples()
            ):
                plays[_uuid_text(aid)] = n
    out = []
    for rid, name, dz, date in stars:
        key = _uuid_text(rid)
        out.append({
            "id": key,
            "deezer_id": str(dz or ""),
            "name": name,
            "picture": f"/api/cover/{key}" if dz else None,
            "tracks": counts.get(key, 0),
            "plays": plays.get(key, 0),
            "starred": date.isoformat() if hasattr(date, "isoformat") else str(date or ""),
        })
    out.sort(key=lambda a: (-a["plays"], (a["name"] or "").lower()))
    return jsonify({"artists": out})


@webapi.route("/me/albums")
@login_required
def my_albums():
    """The albums the person starred, newest star first, as album cards."""
    from ..db import StarredAlbum

    q = (
        Album.select(Album, Artist)
        .join(Artist)
        .switch(Album)
        .join(StarredAlbum, on=(StarredAlbum.starred == Album.id))
        .where(StarredAlbum.user == request.webuser)
        .order_by(StarredAlbum.date.desc())
    )
    return jsonify({"albums": _db_album_cards(q)})


# -- podcasts ---------------------------------------------------------------


def _podcast_owner():
    """Owner of imported podcast channels: the configured sync_user if any, else
    the requesting user."""
    sync_user = current_app.config["DEEZER"].get("sync_user")
    if sync_user:
        try:
            return User.get(name=sync_user)
        except User.DoesNotExist:
            pass
    return request.webuser


def _channel_counts(ids: list) -> dict:
    """{channel id: (episodes, archived episodes)} in one grouped query."""
    out = {}
    archived = fn.SUM(peewee.Case(None, [(PodcastEpisode.path.is_null(False), 1)], 0))
    for i in range(0, len(ids), _IN_CHUNK):
        for cid, nb, arch in (
            PodcastEpisode.select(PodcastEpisode.channel, fn.COUNT(PodcastEpisode.id), archived)
            .where(PodcastEpisode.channel << ids[i : i + _IN_CHUNK])
            .group_by(PodcastEpisode.channel)
            .tuples()
        ):
            out[cid] = (nb, int(arch or 0))
    return out


def _transcript_cards(ids: list) -> dict:
    """{episode id: transcription state} in one grouped query per 400 ids."""
    from ..transcribe import job as _job

    out = {}
    for start in range(0, len(ids), _IN_CHUNK):
        chunk = ids[start : start + _IN_CHUNK]
        for row in (
            PodcastTranscript.select(
                PodcastTranscript.episode,
                PodcastTranscript.status,
                PodcastTranscript.done_ms,
                PodcastTranscript.duration_ms,
                PodcastTranscript.language,
                PodcastTranscript.source,
                PodcastTranscript.error_code,
                PodcastTranscript.n_segments,
            )
            .where(PodcastTranscript.episode << chunk)
            .tuples()
        ):
            eid, status, done, duration, language, source, code, count = row
            if status == "ready":
                progress = 1.0
            elif status in ("running", "leased"):
                progress = _job.progress(eid)
            elif duration:
                progress = max(0.0, min(1.0, int(done or 0) / int(duration)))
            else:
                progress = 0.0
            out[eid] = {
                "status": status,
                "progress": float(progress or 0.0),
                "language": language,
                "source": source,
                "error": {"code": code} if code else None,
                "segment_count": int(count or 0),
            }
    return out


#: One page of a show's episodes: what a phone screen opens with, and the most
#: one answer may carry. A three-year daily show serialized every episode on
#: every open — ~450 B each, unbounded — which is what the page cost.
PODCAST_EPISODES_PAGE = 50
PODCAST_EPISODES_MAX = 200
#: A cursor is a sort key, not a token: bound what is parsed before it is used.
PODCAST_CURSOR_MAX = 96


def _transcribed_count(channel):
    """How many of a show's episodes have a finished transcript, in one query."""
    return (
        PodcastTranscript.select(fn.COUNT(PodcastTranscript.episode))
        .join(PodcastEpisode, on=(PodcastTranscript.episode == PodcastEpisode.id))
        .where(
            (PodcastEpisode.channel == channel.id)
            & (PodcastTranscript.status == "ready")
        )
        .scalar()
    ) or 0


def _episode_key(episode):
    """The opaque keyset cursor of a page: its last episode's sort key."""
    published = episode.publish_date.isoformat() if episode.publish_date else ""
    created = episode.created.isoformat() if episode.created else ""
    return "%s|%s|%s" % (published, created, episode.id)


def _episode_cursor(raw):
    """``(published, created, id)`` from a ``next`` value; ValueError if junk.

    The three parts are exactly what the page is ordered by, so a second page
    never overlaps or skips a row, whatever was published while it was open.
    """
    if raw is None or raw == "":
        return None
    if not isinstance(raw, str) or len(raw) > PODCAST_CURSOR_MAX:
        raise ValueError("invalid cursor")
    parts = raw.split("|")
    if len(parts) != 3:
        raise ValueError("invalid cursor")
    published_text, created_text, id_text = parts
    try:
        episode_id = uuid.UUID(id_text)
        created = (
            datetime.datetime.fromisoformat(created_text) if created_text else None
        )
        published = (
            datetime.datetime.fromisoformat(published_text)
            if published_text
            else None
        )
    except (AttributeError, TypeError, ValueError):
        raise ValueError("invalid cursor")
    if created is None:
        raise ValueError("invalid cursor")
    return (published, created, episode_id)


def _episodes_after(cursor):
    """Rows strictly after ``cursor`` in the page's (published, created, id) order.

    NULL-dated episodes sort last (the page's own ORDER BY), so a cursor on a
    dated row also lets the NULL tail through, and a cursor inside the tail
    only lets the tail rows older than it through.
    """
    published, created, episode_id = cursor
    older = (PodcastEpisode.created < created) | (
        (PodcastEpisode.created == created) & (PodcastEpisode.id < episode_id)
    )
    if published is None:
        return PodcastEpisode.publish_date.is_null(True) & older
    return (
        PodcastEpisode.publish_date.is_null(True)
        | (PodcastEpisode.publish_date < published)
        | ((PodcastEpisode.publish_date == published) & older)
    )


def _episode_limit(raw):
    """``(limit, None)`` or ``(None, response)``: 1..200, 50 by default."""
    if raw is None or raw == "":
        return PODCAST_EPISODES_PAGE, None
    try:
        value = int(str(raw))
    except (TypeError, ValueError):
        return None, (jsonify({"error": "invalid limit"}), 400)
    if value < 1 or value > PODCAST_EPISODES_MAX:
        return None, (jsonify({"error": "invalid limit"}), 400)
    return value, None


def _channel(c, with_episodes=False, counts=None, limit=None, cursor=None):
    """A podcast channel for the SPA. `counts` is (episodes, archived) from
    ``_channel_counts``; without it this channel is counted in one grouped
    query. ``limit`` pages the episodes (newest first) and adds a ``next``
    cursor; without it every episode is listed, as the subscribe answer wants.
    """
    if counts is None:
        counts = _channel_counts([c.id]).get(c.id, (0, 0))
    info = {
        "id": str(c.id),
        "deezer_id": c.deezer_id,
        "title": c.title or "",
        "description": c.description or "",
        # Once the show is gone from Deezer, its picture comes from our own
        # archive (same-origin) — the CDN URL is not ours to rely on.
        "cover": (
            "/api/cover/" + str(c.id)
            if c.gone is not None
            else _image("talk", c.cover_art_md5)
        ),
        "episode_count": counts[0],
        "status": "error" if c.error_message else "ok",
        # False for a show you unsubscribed from but whose episodes are
        # archived: it no longer syncs, and everything downloaded stays yours.
        "subscribed": bool(c.subscribed),
        # True once Deezer stopped serving the show entirely. It becomes a LOCAL
        # podcast: everything archived stays listed and playable, served from
        # disk, cover included. Nothing about it depends on Deezer any more.
        "local": c.gone is not None,
        "archived_count": counts[1],
        # The show's own "archive new episodes automatically" switch: what the
        # archive keeps of it (backfill.collect / archive_show).
        "auto_archive": bool(c.auto_archive),
        # Its transcription switch and language override (NULL means auto).
        "auto_transcribe": bool(c.auto_transcribe),
        "transcribe_language": c.transcribe_language or "auto",
    }
    if with_episodes:
        from ..deezer.chapters import counts_for as _chapter_counts

        query = c.episodes.order_by(
            # NULL-dated episodes last on every engine: PostgreSQL sorts them
            # first under DESC, which a keyset cursor cannot follow.
            PodcastEpisode.publish_date.is_null().asc(),
            PodcastEpisode.publish_date.desc(),
            PodcastEpisode.created.desc(),
            PodcastEpisode.id.desc(),
        )
        if cursor is not None:
            query = query.where(_episodes_after(cursor))
        page = list(query if limit is None else query.limit(int(limit) + 1))
        has_more = limit is not None and len(page) > int(limit)
        episodes = page[: int(limit)] if limit is not None else page
        chapter_counts = _chapter_counts([e.id for e in episodes])
        transcripts = _transcript_cards([e.id for e in episodes])
        if has_more:
            # The page cannot say how much of the show is transcribed: ask the
            # show once, so the counter does not reset at each page boundary.
            info["transcribed_count"] = _transcribed_count(c)
        else:
            info["transcribed_count"] = sum(
                1 for card in transcripts.values() if card["status"] == "ready"
            )
        info["episodes"] = [
            _episode(
                e,
                c,
                chapter_count=chapter_counts.get(e.id, 0),
                transcript=transcripts.get(e.id),
            )
            for e in episodes
        ]
        if limit is not None:
            info["next"] = _episode_key(episodes[-1]) if has_more else None
    return info


def _episode(e, channel=None, chapter_count=0, transcript=None):
    """A podcast episode shaped like a playable track for the web player.

    Its universal stream id (``deezer_id``) is the episode UUID — the same
    convention local tracks use — so the existing queue/player machinery plays
    it unchanged via ``/api/stream/<uuid>``. ``chapter_count`` is the number
    of chapters the archived file carries (0 when there is none); the list
    itself is only fetched when a person expands the episode. ``transcript``
    is the episode's transcription state, or None when it has never been asked
    for — never its sidecar path.
    """
    channel = channel or e.channel
    archived = bool(e.path and os.path.isfile(e.path))
    # A show Deezer has dropped can only serve what we archived; the rest is
    # gone for good and says so rather than failing on play.
    gone = channel.gone is not None and not archived
    cover = (
        "/api/cover/" + str(channel.id)
        if channel.gone is not None
        else _image("talk", e.image_md5 or channel.cover_art_md5)
    )
    return {
        "id": str(e.id),
        "deezer_id": str(e.id),
        "podcast": True,
        "title": e.title,
        "description": e.description or "",
        "duration": e.duration or 0,
        "published": int(e.publish_date.timestamp()) if e.publish_date else 0,
        "status": e.status,
        "archived": archived,
        "unavailable": gone,
        "explicit": False,
        "chapter_count": int(chapter_count or 0),
        "transcript": transcript,
        "channel_id": str(channel.id),
        "artist": {"deezer_id": str(channel.id), "name": channel.title or ""},
        "album": {
            "deezer_id": str(channel.id),
            "title": channel.title or "",
            "cover": cover,
        },
        "cover": cover,
    }


@webapi.route("/podcasts")
@login_required
def podcasts():
    # Unsubscribed-but-archived shows are still listed (flagged `subscribed:
    # false`): the audio is on the server and must stay reachable — the whole
    # point of archiving is that leaving Deezer's catalogue changes nothing.
    channels = list(PodcastChannel.select().order_by(fn.lower(PodcastChannel.title)))
    counts = _channel_counts([c.id for c in channels])
    return jsonify({"podcasts": [_channel(c, counts=counts.get(c.id, (0, 0))) for c in channels]})


@webapi.route("/podcast/<pid>")
@login_required
def podcast(pid):
    try:
        c = PodcastChannel[uuid.UUID(str(pid))]
    except ValueError:
        # A catalogue card names its show by Deezer's id; a UUID cannot be
        # turned back into one, so the page opens by either form.
        c = (
            PodcastChannel.get_or_none(PodcastChannel.deezer_id == str(pid))
            if _valid_id(str(pid))
            else None
        )
        if c is None:
            return jsonify({"error": "not found"}), 404
    except PodcastChannel.DoesNotExist:
        return jsonify({"error": "not found"}), 404
    limit, error = _episode_limit(request.args.get("limit"))
    if error:
        return error
    try:
        cursor = _episode_cursor(request.args.get("cursor"))
    except ValueError:
        return jsonify({"error": "invalid cursor"}), 400
    return jsonify(_channel(c, with_episodes=True, limit=limit, cursor=cursor))


@webapi.route("/podcasts", methods=["POST"])
@login_required
def subscribe_podcast():
    if not _is_admin():
        return jsonify({"error": "forbidden"}), 403
    provider, err = _need_provider()
    if err:
        return err
    data = request.get_json(silent=True) or {}
    url = (data.get("url") or "").strip()
    if not url:
        return jsonify({"error": "missing url"}), 400

    from ..deezer.archive import parse_deezer_ref, import_show

    try:
        kind, did = parse_deezer_ref(url)
    except ValueError:
        return jsonify({"error": "unsupported url"}), 400
    if kind not in ("show", "episode"):
        return jsonify({"error": "not a podcast url"}), 400

    show_id = did
    if kind == "episode":
        try:
            info = provider.dz.api.get_episode(did)
            show_id = (info.get("podcast") or {}).get("id") or info.get("podcast_id")
        except Exception as exc:
            if _deezer_down(exc):
                return _deezer_unreachable()
            return jsonify({"error": "could not resolve show"}), 502
        if not show_id:
            return jsonify({"error": "could not resolve show"}), 400

    cfg = current_app.config["DEEZER"]
    known = PodcastChannel.get_or_none(PodcastChannel.deezer_id == str(show_id))
    fresh = known is None or not known.subscribed
    try:
        c = import_show(
            provider, _podcast_owner(), show_id,
            episode_limit=int(cfg.get("podcast_episodes") or 30),
        )
    except Exception:
        logger.warning("Podcast import failed for %s", show_id, exc_info=True)
        return jsonify({"error": "import failed"}), 502
    from ..deezer import backfill, rules

    app = current_app._get_current_object()
    if fresh:
        # A new subscription starts with the admin's default ("Abonnement à un
        # podcast" in the archiving rules); a show already followed keeps the
        # switch someone set on its page.
        c.auto_archive = rules.enabled(app, "on_podcast")
        c.save()
    # Subscribing is the event: with the switch on, every episode we just
    # imported gets archived, so the show survives leaving Deezer.
    backfill.archive_show(app, c)
    return jsonify(_channel(c, with_episodes=True))


@webapi.route("/podcast/<pid>/auto_archive", methods=["POST"])
@login_required
def podcast_auto_archive(pid):
    """A show's "archive new episodes automatically" switch. Turning it on also
    queues the episodes already listed that have no file yet: they go through
    the archiver's background queue one at a time (BULK, behind anything
    played), so a show of 40 episodes never competes with listening."""
    if not _is_admin():
        return jsonify({"error": "forbidden"}), 403
    try:
        c = PodcastChannel[uuid.UUID(str(pid))]
    except (ValueError, PodcastChannel.DoesNotExist):
        return jsonify({"error": "not found"}), 404
    on = (request.get_json(silent=True) or {}).get("on")
    if not isinstance(on, bool):
        return jsonify({"error": "on must be true or false"}), 400
    c.auto_archive = on
    c.save()
    queued = 0
    if on:
        from ..deezer import backfill

        queued = backfill.archive_show(current_app._get_current_object(), c)
    return jsonify({**_channel(c), "queued": queued})


@webapi.route("/podcast/<pid>", methods=["DELETE"])
@login_required
def unsubscribe_podcast(pid):
    if not _is_admin():
        return jsonify({"error": "forbidden"}), 403
    try:
        c = PodcastChannel[uuid.UUID(str(pid))]
    except (ValueError, PodcastChannel.DoesNotExist):
        return jsonify({"error": "not found"}), 404

    # Unsubscribing stops the subscription — it does NOT destroy what has
    # already been archived. An episode on disk is yours: it survives Deezer
    # dropping the show, and it survives this. So a channel with archived audio
    # is kept and simply marked unsubscribed (it stops syncing and leaves the
    # subscribed list, while everything downloaded stays playable); only a
    # channel with nothing on disk is actually removed, since there is nothing
    # of yours to lose.
    archived = [
        e for e in c.episodes if e.path and os.path.isfile(e.path)
    ]
    if archived:
        c.subscribed = False
        c.save()
        return jsonify({"ok": True, "kept": len(archived), "archived": True})
    c.delete_instance(recursive=True)
    return ("", 204)


# -- podcast progress & markers ----------------------------------------------
# Per-user, server-side: an episode resumes where THIS user left it on any
# device, and manual markers (bookmarked positions) follow them everywhere.


# How close to the end (seconds) counts as "finished listening".
_EPISODE_DONE_TAIL = 15
# Sanity cap on user-supplied positions/durations (seconds).
_MAX_POSITION = 10**7
# Cap markers per user per episode so a hostile client can't grow the table.
_MAX_MARKERS = 100
# ...and cap the totals per user: the per-episode cap alone bounds nothing,
# because the number of episodes a client can name is itself unbounded.
_MAX_MARKERS_PER_USER = 5000
_MAX_PROGRESS_PER_USER = 10000


def _episode_by_id(eid):
    try:
        return PodcastEpisode[uuid.UUID(str(eid))]
    except (ValueError, PodcastEpisode.DoesNotExist):
        return None


@webapi.route("/podcast/progress")
@login_required
def podcast_progress():
    """All of this user's episode positions, keyed by episode UUID."""
    out = {}
    q = PodcastProgress.select().where(PodcastProgress.user == request.webuser)
    for row in q:
        out[str(row.episode_id)] = {
            "position": row.position,
            "duration": row.duration,
            "finished": row.finished,
            "updated": int(row.updated.timestamp()),
        }
    return jsonify({"progress": out})


@webapi.route("/podcast/progress", methods=["POST"])
@login_required
def save_podcast_progress():
    """Upsert this user's position in an episode (the player calls it
    periodically while playing, and on pause/track change/page hide)."""
    data = request.get_json(silent=True) or {}
    episode = _episode_by_id(data.get("episode_id"))
    if episode is None:
        return jsonify({"error": "not found"}), 404
    try:
        position = int(float(data.get("position") or 0))
        duration = int(float(data.get("duration") or 0))
    except (TypeError, ValueError):
        return jsonify({"error": "invalid position"}), 400
    if not (0 <= position <= _MAX_POSITION) or not (0 <= duration <= _MAX_POSITION):
        return jsonify({"error": "invalid position"}), 400
    duration = duration or episode.duration or 0
    finished = bool(data.get("finished"))
    # Reaching the last seconds counts as finished even if the client didn't
    # say so — a completed episode must not offer to resume from the credits.
    if duration and position >= max(0, duration - _EPISODE_DONE_TAIL):
        finished = True

    row = PodcastProgress.get_or_none(
        PodcastProgress.user == request.webuser, PodcastProgress.episode == episode
    )
    if row is None:
        # Only a NEW row can grow the table, so the cap is checked here alone —
        # updating an existing position stays free.
        total = (
            PodcastProgress.select()
            .where(PodcastProgress.user == request.webuser)
            .count()
        )
        if total >= _MAX_PROGRESS_PER_USER:
            return jsonify({"error": "too many saved positions"}), 429
        PodcastProgress.create(
            user=request.webuser,
            episode=episode,
            position=position,
            duration=duration,
            finished=finished,
        )
        created = True
    else:
        created = False
    if not created:
        row.position = position
        row.duration = duration
        row.finished = finished
        row.updated = now()
        row.save()

    # The admin's position also goes into the legacy per-episode offset
    # (Subsonic bookmarkPosition). It stays on this server.
    if _is_admin():
        episode.play_offset = 0 if finished else position
        episode.save(only=(PodcastEpisode.play_offset,))
    return jsonify({"ok": True, "finished": finished})


def _marker_rows(where):
    return (
        PodcastMarker.select()
        .where(PodcastMarker.user == request.webuser, where)
        .order_by(PodcastMarker.position)
    )


@webapi.route("/podcast/<pid>/markers")
@login_required
def channel_markers(pid):
    """This user's markers across a whole show, grouped by episode — one call
    for the show page instead of one per episode."""
    try:
        channel = PodcastChannel[uuid.UUID(str(pid))]
    except (ValueError, PodcastChannel.DoesNotExist):
        return jsonify({"error": "not found"}), 404
    episodes = PodcastEpisode.select(PodcastEpisode.id).where(
        PodcastEpisode.channel == channel
    )
    out = {}
    for m in _marker_rows(PodcastMarker.episode.in_(episodes)):
        out.setdefault(str(m.episode_id), []).append(m.responsize())
    return jsonify({"markers": out})


@webapi.route("/podcast/episode/<eid>/markers")
@login_required
def episode_markers(eid):
    episode = _episode_by_id(eid)
    if episode is None:
        return jsonify({"error": "not found"}), 404
    rows = _marker_rows(PodcastMarker.episode == episode)
    return jsonify({"markers": [m.responsize() for m in rows]})


@webapi.route("/podcast/episode/<eid>/markers", methods=["POST"])
@login_required
def add_episode_marker(eid):
    episode = _episode_by_id(eid)
    if episode is None:
        return jsonify({"error": "not found"}), 404
    data = request.get_json(silent=True) or {}
    try:
        position = int(float(data.get("position")))
    except (TypeError, ValueError):
        return jsonify({"error": "invalid position"}), 400
    if not (0 <= position <= _MAX_POSITION):
        return jsonify({"error": "invalid position"}), 400
    label = data.get("label")
    label = str(label).strip()[:120] if label else None
    count = (
        PodcastMarker.select()
        .where(
            PodcastMarker.user == request.webuser, PodcastMarker.episode == episode
        )
        .count()
    )
    if count >= _MAX_MARKERS:
        return jsonify({"error": "too many markers"}), 400
    # The per-episode cap bounds nothing on its own — the client picks the
    # episode, so it can spread unlimited markers over unlimited episodes.
    total = (
        PodcastMarker.select().where(PodcastMarker.user == request.webuser).count()
    )
    if total >= _MAX_MARKERS_PER_USER:
        return jsonify({"error": "too many markers"}), 400
    m = PodcastMarker.create(
        user=request.webuser, episode=episode, position=position, label=label
    )
    return jsonify({"ok": True, "marker": m.responsize()})


@webapi.route("/podcast/marker/<mid>", methods=["DELETE"])
@login_required
def delete_episode_marker(mid):
    try:
        m = PodcastMarker[uuid.UUID(str(mid))]
    except (ValueError, PodcastMarker.DoesNotExist):
        return jsonify({"error": "not found"}), 404
    # Markers are personal: another user's marker doesn't exist for you.
    if m.user_id != request.webuser.id:
        return jsonify({"error": "not found"}), 404
    m.delete_instance()
    return ("", 204)


# -- favorites & playback ---------------------------------------------------


_LISTEN_MIN_SECONDS = 20


def _record_play(track, listened):
    """Count a play locally: ``play_count`` / ``last_play`` on the Track row.

    Subsonic's ``scrobble`` was the only thing writing these, so a library
    played entirely through the web app looked untouched — which matters now
    that the archive cleanup decides what to drop from exactly this data. A skip
    is not a play (hence the floor), and this never touches Deezer.
    """
    if track is None:
        return
    # The person's own history keeps every change, skips included: they are
    # what the radios and the Flow learn to steer away from.
    from .. import listening

    ctx = (request.get_json(silent=True) or {}).get("context") or {}
    listening.record(request.webuser, track, listened,
                     ctx.get("kind") if isinstance(ctx, dict) else None)
    if listened < _LISTEN_MIN_SECONDS:
        return
    try:
        Track.update(play_count=Track.play_count + 1, last_play=now()).where(
            Track.id == track.id
        ).execute()
    except Exception:  # a stats write must never break playback
        logger.debug("Could not record a play for %s", track.id, exc_info=True)


@webapi.route("/listen", methods=["POST"])
@login_required
def report_listen():
    """A track finished playing: count it locally. Nothing is reported to Deezer.

    The web player calls this on every track change, so it stays cheap: one
    indexed lookup and one update."""
    from . import refs

    data = request.get_json(silent=True) or {}
    raw = str(data.get("deezer_id") or data.get("id") or "")
    track = refs.resolve_track(raw)
    if track is None and not _valid_id(raw):
        try:
            uid = uuid.UUID(raw)
        except ValueError:
            return jsonify({"error": "invalid deezer_id"}), 400
        # ``refs`` hides a row the caller may not read, so an upload that
        # belongs to somebody else comes back as None. It still EXISTS, and a
        # listen on it must be a 404, not a silent 204.
        hidden = Track.get_or_none(Track.id == uid)
        if hidden is not None and not _may_access_track(hidden):
            return jsonify({"error": "not found"}), 404
    if track is not None and not _may_access_track(track):
        # Another person's private upload does not exist for this account: a
        # listen reported on it would write a history row against a file they
        # may not read.
        return jsonify({"error": "not found"}), 404
    try:
        listened = int(data.get("listened") or 0)
    except (TypeError, ValueError):
        listened = 0
    _record_play(track, listened)

    # Playing from an album or a playlist can archive the whole thing — off by
    # default; see rules.on_play_context.
    from ..deezer import backfill

    backfill.archive_play_context(
        current_app._get_current_object(), _provider(), data.get("context") or {}
    )
    return ("", 204)


def _set_star(track, on, star=None):
    """Star/unstar a row locally for the current web user (a track unless
    ``star`` names another starred model)."""
    star = star or StarredTrack
    if on:
        try:
            star[request.webuser.id, track.id]
        except star.DoesNotExist:
            star.create(user=request.webuser, starred=track, date=now())
    else:
        star.delete().where(
            star.user == request.webuser, star.starred == track.id
        ).execute()


@webapi.route("/favorite", methods=["POST"])
@login_required
def favorite():
    from . import refs

    data = request.get_json(silent=True) or {}
    raw = str(data.get("deezer_id") or data.get("id") or "")
    on = bool(data.get("on", True))

    # Stars are local: nothing is written to the Deezer account. A track we know
    # (a row UUID, a Deezer id already imported, a remembered YouTube hit) is
    # starred from the database alone, so it works with Deezer unreachable.
    track = refs.resolve_track(raw, materialize=True)
    if track is None:
        if not _valid_id(raw):
            return jsonify({"error": "invalid deezer_id"}), 400
        # A Deezer track we never imported: its metadata row is a catalogue
        # lookup, the one thing here that needs Deezer.
        from ..deezer import archive

        provider = _provider()
        if provider is None:
            return _deezer_disabled()
        try:
            track = archive.import_track(provider, raw)
        except Exception:
            logger.warning("favorite: metadata fetch failed for %s", int(raw), exc_info=True)
            return jsonify({"error": "track unavailable"}), 502
    _set_star(track, on)
    if on:
        # Starring it puts it in your library, so archive it now rather than
        # hoping you press play before Deezer drops it. Unstarring archives
        # nothing and, as always, deletes nothing.
        from ..deezer import backfill

        backfill.archive_tracks(
            current_app._get_current_object(), [track], event="on_fav_track"
        )
    out = {"ok": True, "favorite": on}
    if not _valid_id(raw):
        out["local"] = True
    return jsonify(out)


# What a favourite of each kind is stored against: the model, its starred
# table and the deterministic id the importer would give the row.
def _star_kinds():
    from ..db import StarredAlbum, StarredArtist, StarredPlaylist
    from ..deezer import ids as dz_ids

    return {
        "album": (Album, StarredAlbum, dz_ids.album_uuid),
        "artist": (Artist, StarredArtist, dz_ids.artist_uuid),
        "playlist": (Playlist, StarredPlaylist, dz_ids.playlist_uuid),
    }


def _starred(kind, row=None, deezer_id=None) -> bool:
    """Whether the current user starred this album / artist / playlist: one
    indexed statement, no call to Deezer. The entity is given as a row or by the
    Deezer id the page was opened with."""
    model, star, make_uuid = _star_kinds()[kind]
    user = getattr(request, "webuser", None)
    if user is None:
        return False
    if row is not None:
        cond = star.starred == row.id
    elif _valid_id(deezer_id):
        cond = (model.deezer_id == str(deezer_id)) | (model.id == make_uuid(deezer_id))
    else:
        return False
    return (
        star.select()
        .join(model, on=(star.starred == model.id))
        .where(star.user == user.id, cond)
        .exists()
    )


def _name_field(data, key):
    value = data.get(key)
    if not isinstance(value, str):
        return None
    value = value.strip()[:256]
    return value or None


def _stub_entity(kind, deezer_id, data):
    """The row a favourite of an album / artist hangs on, made from what the
    client knows when we never imported it (a stub: the importer or the archive
    fills it in later). ``None`` when the body does not name it."""
    from ..deezer import library

    if kind == "artist":
        name = _name_field(data, "title") or _name_field(data, "artist")
        if not name:
            return None
        return library.upsert_artist(deezer_id, name)
    title = _name_field(data, "title")
    artist_name = _name_field(data, "artist")
    if not title or not artist_name:
        return None
    artist_id = data.get("artist_id")
    if _valid_id(artist_id):
        artist = library.upsert_artist(str(artist_id), artist_name)
    else:
        artist = Artist.select().where(Artist.name == artist_name).first() or Artist.create(
            name=artist_name
        )
    return library.upsert_album(deezer_id, title, artist, None)


def _star_playlist_when_there(app, provider, deezer_id, user_id, user_name, title):
    """Worker: import a Deezer playlist nobody here has, then star it for
    ``user_id``. The star of a playlist that was never imported therefore shows
    up once the import is done (a few seconds), not at the click."""
    from ..db import StarredPlaylist, User, close_connection, open_connection
    from ..deezer import ids as dz_ids
    from ..deezer.importer import DeezerImporter

    try:
        with app.app_context():
            open_connection(reuse=True)
            try:
                raw = provider.get_playlist_tracks(deezer_id)
                importer = DeezerImporter(provider, user_name)
                name = title
                if not name:
                    page = provider.dz.gw.get_playlist_page(deezer_id)
                    name = ((page or {}).get("DATA") or {}).get("TITLE") or "Playlist"
                pl, _ = importer._materialize_playlist(
                    dz_ids.playlist_uuid(deezer_id), name, deezer_id, raw
                )
                user = User.get_or_none(User.id == user_id)
                if user is not None and not StarredPlaylist.select().where(
                    StarredPlaylist.user == user, StarredPlaylist.starred == pl
                ).exists():
                    StarredPlaylist.create(user=user, starred=pl, date=now())
            finally:
                close_connection()
    except Exception:
        _log_deezer_failure("import of favourited playlist %s failed", deezer_id)


@webapi.route("/favorite/<kind>", methods=["POST"])
@login_required
def favorite_entity(kind):
    """Star / unstar an album, artist or playlist for the current user.

    Local, like the track stars: no write to the Deezer account. Archiving what
    it holds stays the admin's (it spends the shared disk and Deezer session)."""
    kinds = _star_kinds()
    if kind not in kinds:
        return jsonify({"error": "unknown favorite kind"}), 400
    model, star, make_uuid = kinds[kind]
    data = request.get_json(silent=True) or {}
    raw = str(data.get("deezer_id") or data.get("id") or "")
    on = bool(data.get("on", True))
    row = None
    if _valid_id(raw):
        row = (
            model.select()
            .where((model.deezer_id == raw) | (model.id == make_uuid(raw)))
            .first()
        )
        deezer_id = raw
    else:
        try:
            row = model.get_or_none(model.id == uuid.UUID(raw))
        except ValueError:
            return jsonify({"error": "invalid deezer_id"}), 400
        deezer_id = row.deezer_id if row is not None else None
        if row is None:
            return jsonify({"error": "not found"}), 404
    provider = _provider()
    app = current_app._get_current_object()
    pending = False
    if row is None:
        if kind == "playlist":
            if not on:
                return jsonify({"ok": True, "favorite": False})
            if provider is None:
                return _deezer_disabled()
            pending = True
        else:
            if not on:
                return jsonify({"ok": True, "favorite": False})
            row = _stub_entity(kind, deezer_id, data)
            if row is None:
                return jsonify({"error": "missing title"}), 400
    if row is not None:
        if kind == "playlist" and row.user_id != request.webuser.id and not row.public and not _is_admin():
            return jsonify({"error": "not found"}), 404
        _set_star(row, on, star)
    if on and deezer_id and _is_admin():
        # A favourited album, playlist or artist is now yours: archive everything
        # it holds, in the background. For an artist that means the FULL
        # discography, fed release by release (see backfill.archive_entity).
        from ..deezer import backfill

        backfill.archive_entity(app, provider, kind, deezer_id)
    if pending:
        args = (
            app, provider, deezer_id, request.webuser.id, request.webuser.name,
            _name_field(data, "title"),
        )
        if current_app.testing:
            _star_playlist_when_there(*args)
        else:
            threading.Thread(
                target=_star_playlist_when_there, args=args,
                name="star-playlist", daemon=True,
            ).start()
    return jsonify({"ok": True, "favorite": on})


# -- playlist management -----------------------------------------------------
# All edits act on the DB Playlist (so local files are first-class), under the
# role matrix of ``neezer/playlist_access.py``. A playlist id is the Playlist
# UUID; a Deezer numeric id of an imported playlist also resolves. Nothing is
# written to a provider account — there is no mirror any more.
#
# The order-sensitive mutations are compared-and-swapped on ``Playlist.version``
# inside the same transaction as the track write: a stale client gets 409 + the
# authoritative counters, never a silently merged order. ``version`` is OPTIONAL
# on every route so an installed older SPA (which cannot send it) keeps working.

#: A title or a description is one 256-char database column.
PLAYLIST_MAX_TITLE = 256
PLAYLIST_MAX_DESCRIPTION = 256
#: One request may carry this many track refs / indexes. A reorder carries the
#: WHOLE visible list (a 4 000-title playlist), so it gets its own, larger cap;
#: the 256 KiB body cap still bounds the request.
PLAYLIST_MAX_TRACKS = 2000
PLAYLIST_MAX_ORDER = 10000
#: One account's playlists. Creation is open to every account, so a scripted
#: client must not be able to fill the table; 500 is far above a real library.
PLAYLIST_MAX_PER_USER = 500
#: A visible-list index is bounded before it is ever used.
PLAYLIST_MAX_INDEX = 10000
#: Body caps (413): a 4 000-ref order of UUIDs is ~160 KiB; a member/metadata
#: body is a few dozen bytes.
PLAYLIST_BODY_TRACKS = 256 * 1024
PLAYLIST_BODY_METADATA = 16 * 1024
PLAYLIST_MAX_VERSION = 2**31 - 1

_LIMITERS = {}


def _reset_playlist_limits():
    """Drop the process-wide creation windows (tests only).

    Production keeps them: a module-level limiter is the point (one gunicorn
    worker, one view). A test that creates dozens of playlists in one process
    calls this, like ``playlist_share._stream_limiter.clear()``.
    """
    _LIMITERS.pop("playlist-create", None)


def _window_limiter(name, max_events, window):
    """The edge lot's ``WindowLimiter`` when it is installed, else None.

    ``neezer/ratelimit.py`` is owned by the edge lot (it lands the sliding
    window used by creation and membership). Importing it lazily keeps this
    module runnable before that file exists, and picks the real limiter up as
    soon as it does — no duplicate counter, no second policy.
    """
    limiter = _LIMITERS.get(name)
    if limiter is not None:
        return limiter
    try:
        from ..ratelimit import WindowLimiter
    except ImportError:
        return None
    limiter = WindowLimiter(max_events, window)
    _LIMITERS[name] = limiter
    return limiter


def _rate_limited(limiter, key):
    """``None`` when the call may pass, else the 429 response."""
    if limiter is None:
        return None
    allowed, retry_after = limiter.allow(key)
    if allowed:
        return None
    delay = max(1, int(float(retry_after or 0) + 0.999))
    response = jsonify({"error": "rate limited", "retry_after": delay})
    response.status_code = 429
    response.headers["Retry-After"] = str(delay)
    return response


def _playlist_body(max_bytes, allow_empty=False):
    """``(dict, None)`` or ``(None, response)``: 413 over the cap, 400 junk.

    An empty body is a 400 (the /api coverage walk POSTs every route with no
    body at all) unless the route explicitly accepts ``{}`` (leave, the remove
    of a member). Never a 500.

    A declared ``Content-Length`` is checked before anything is read. A chunked
    body has none, so it is read through a bounded read instead: a hostile
    client must not be able to make the worker buffer an unbounded JSON body
    just by omitting the length.
    """
    given = request.content_length
    if given is not None and given > max_bytes:
        return None, (jsonify({"error": "body too large"}), 413)
    if given is None:
        try:
            raw = request.stream.read(max_bytes + 1)
        except Exception:  # a client that disconnects mid-body
            return None, (jsonify({"error": "invalid body"}), 400)
        if len(raw) > max_bytes:
            return None, (jsonify({"error": "body too large"}), 413)
        if not raw.strip():
            if allow_empty:
                return {}, None
            return None, (jsonify({"error": "invalid body"}), 400)
        try:
            data = json.loads(raw)
        except (ValueError, UnicodeDecodeError):
            return None, (jsonify({"error": "invalid body"}), 400)
    else:
        data = request.get_json(silent=True)
    if data is None and allow_empty:
        return {}, None
    if not isinstance(data, dict):
        return None, (jsonify({"error": "invalid body"}), 400)
    return data, None


def _playlist_title(data, required=True):
    """``(title, None)`` or ``(None, response)``: a stripped 1..256 string."""
    if "title" not in data:
        if required:
            return None, (jsonify({"error": "missing title"}), 400)
        return None, None
    raw = data.get("title")
    if not isinstance(raw, str):
        return None, (jsonify({"error": "invalid title"}), 400)
    title = raw.strip()
    if not title:
        return None, (jsonify({"error": "missing title"}), 400)
    if len(title) > PLAYLIST_MAX_TITLE:
        return None, (jsonify({"error": "title too long"}), 400)
    return title, None


def _playlist_description(data):
    """``(comment, None)`` or ``(None, response)``; absent stays absent."""
    if "description" not in data:
        return None, None
    raw = data.get("description")
    if raw is None:
        return None, None
    if not isinstance(raw, str):
        return None, (jsonify({"error": "invalid description"}), 400)
    if len(raw) > PLAYLIST_MAX_DESCRIPTION:
        return None, (jsonify({"error": "description too long"}), 400)
    return raw or None, None


def _playlist_refs(data, key="tracks", max_refs=PLAYLIST_MAX_TRACKS):
    """``([ref], None)`` or ``(None, response)``: ≤ ``max_refs`` track ids."""
    raw = data.get(key)
    if raw is None:
        return [], None
    if not isinstance(raw, list):
        return None, (jsonify({"error": "invalid tracks"}), 400)
    if len(raw) > max_refs:
        return None, (jsonify({"error": "too many tracks"}), 400)
    return [str(value) for value in raw], None


def _playlist_expected(data, key):
    """``(version, None)``, ``(None, None)`` when absent, or a 400.

    A bool is not a version (``True`` is an int in Python); neither is a
    string, a float or a non-positive value.
    """
    if key not in data:
        return None, None
    value = data.get(key)
    if (
        isinstance(value, bool)
        or not isinstance(value, int)
        or value < 1
        or value > PLAYLIST_MAX_VERSION
    ):
        return None, (jsonify({"error": "invalid " + key}), 400)
    return value, None


def _playlist_indexes(data):
    """``([index], None)`` or ``(None, response)``: deduplicated 0..10 000."""
    raw = data.get("indexes")
    if not isinstance(raw, list):
        return None, (jsonify({"error": "invalid indexes"}), 400)
    if len(raw) > PLAYLIST_MAX_TRACKS:
        return None, (jsonify({"error": "too many indexes"}), 400)
    out = []
    seen = set()
    for value in raw:
        if (
            isinstance(value, bool)
            or not isinstance(value, int)
            or value < 0
            or value > PLAYLIST_MAX_INDEX
        ):
            return None, (jsonify({"error": "invalid indexes"}), 400)
        if value not in seen:
            seen.add(value)
            out.append(value)
    return out, None


def _addable_tracks(provider, refs):
    """The Track rows of a client's refs that the caller may put in a playlist.

    ``refs.resolve_tracks`` answers every known row without filtering, so the
    private-upload rule is applied here: another account's upload is not a
    track this person can add, and an unknown id is simply left out (a 400 "no
    tracks" when nothing remains). Deezer ids resolve to shared-library rows.
    """
    user = getattr(request, "webuser", None)
    return [track for track in _resolve_tracks(provider, refs) if track.readable_by(user)]


def _reordered_rows(pl, user, order):
    """Rebuild the whole tracklist: the client's visible order, hidden anchored.

    The client sends the ids of the rows it SEES, with occurrence counts (a
    playlist may hold the same title twice). Rows it omitted are appended in
    their stored order, so a stale/partial list is never destructive. A row
    this person may not see (somebody else's private upload) is kept in place
    relative to the visible row it followed — the front when none — so an
    editor can neither move nor delete it by leaving it out.
    """
    stored = list(
        PlaylistTrack.select(
            PlaylistTrack.id,
            PlaylistTrack.index,
            Track.id,
            Track.deezer_id,
            Track.owner,
        )
        .join(Track, on=(PlaylistTrack.track == Track.id))
        .where(PlaylistTrack.playlist == pl.id)
        .order_by(PlaylistTrack.index)
        .tuples()
    )
    is_admin = bool(getattr(user, "admin", False))
    front, buckets, visible = [], {}, []
    pending, last_ptid = [], None
    for ptid, _index, tid, dz, owner in stored:
        if owner is not None and not (is_admin or owner == user.id):
            pending.append(tid)
            continue
        if last_ptid is None:
            front = pending
        elif pending:
            buckets.setdefault(last_ptid, []).extend(pending)
        pending = []
        visible.append((ptid, tid, dz))
        last_ptid = ptid
    if pending:  # trailing hidden rows follow the last visible one
        if last_ptid is None:
            front = pending
        else:
            buckets.setdefault(last_ptid, []).extend(pending)

    by_uid = {}
    for ptid, tid, dz in visible:
        by_uid.setdefault(str(dz) if dz else str(tid), []).append((ptid, tid))
        by_uid.setdefault(str(tid), []).append((ptid, tid))
    from collections import Counter

    avail = Counter(tid for _ptid, tid, _dz in visible)
    used, picked = Counter(), set()
    picks = []
    for uid in order:
        for ptid, tid in by_uid.get(uid, []):
            if ptid in picked or used[tid] >= avail[tid]:
                continue
            picks.append((ptid, tid))
            picked.add(ptid)
            used[tid] += 1
            break
    for ptid, tid, _dz in visible:
        if ptid not in picked:
            picks.append((ptid, tid))
            picked.add(ptid)

    rows = []
    for tid in front:
        rows.append((pl.id, tid, len(rows)))
    for ptid, tid in picks:
        rows.append((pl.id, tid, len(rows)))
        for hidden_tid in buckets.get(ptid, ()):
            rows.append((pl.id, hidden_tid, len(rows)))
    return rows


@webapi.route("/playlists", methods=["POST"])
@login_required
def create_playlist():
    """Create a playlist. Open to every account (Subsonic always was).

    A scripted client cannot fill the table: 500 playlists per account (409) and
    60 creations an hour (429 when the edge lot's limiter is installed). The
    creator gets an ``owner`` member row in the same transaction, so the list
    and the member sheet agree from the first request.
    """
    from .playlist_folders import place_new_playlist, resolve_folder

    user = request.webuser
    limited = _rate_limited(
        _window_limiter("playlist-create", 60, 3600), f"playlist-create:{user.id}"
    )
    if limited is not None:
        return limited
    data, error = _playlist_body(PLAYLIST_BODY_TRACKS)
    if error:
        return error
    title, error = _playlist_title(data)
    if error:
        return error
    description, error = _playlist_description(data)
    if error:
        return error
    refs, error = _playlist_refs(data)
    if error:
        return error
    # "Nouvelle playlist ici": the optional `folder` is resolved BEFORE anything
    # is written, so a foreign or malformed folder is a 404/400 that leaves no
    # half-created playlist behind. Its presence is what asks for a placement
    # (a `null` means "top of the root"); a request from an older SPA without
    # the key keeps its behaviour, with no placement row at all.
    folder = None
    wants_placement = "folder" in data
    if wants_placement:
        folder, error = resolve_folder(user, data.get("folder"))
        if error:
            return error
    provider = _provider()
    track_rows = _addable_tracks(provider, refs)
    with db.atomic():
        if Playlist.select().where(Playlist.user == user).count() >= PLAYLIST_MAX_PER_USER:
            return jsonify({"error": "playlist limit"}), 409
        pl = Playlist.create(user=user, name=title, comment=description)
        PlaylistMember.create(
            playlist=pl, user=user, role="owner", added=pl.created
        )
        pl.add_many(track_rows)
        if wants_placement:
            place_new_playlist(user, pl, folder)
    from ..deezer import backfill

    backfill.archive_tracks(
        current_app._get_current_object(), track_rows, event="on_playlist_add"
    )
    return jsonify(
        {
            "ok": True,
            "id": str(pl.id),
            "deezer_id": pl.deezer_id,
            "folder_id": str(folder.id) if folder is not None else None,
            "version": pl.version,
            "members_version": pl.members_version,
            "role": "owner",
        }
    )


@webapi.route("/playlist/<playlist_id>", methods=["PATCH"])
@login_required
def edit_playlist(playlist_id):
    """Rename / describe a playlist. Owner or admin only; editor gets 403."""
    pl, _role, error = _playlist_guard(playlist_id, "manage")
    if error:
        return error
    data, error = _playlist_body(PLAYLIST_BODY_METADATA)
    if error:
        return error
    expected, error = _playlist_expected(data, "version")
    if error:
        return error
    title, error = _playlist_title(data, required=False)
    if error:
        return error
    description, error = _playlist_description(data)
    if error:
        return error
    fields = {}
    if "title" in data:
        fields["name"] = title
    if "description" in data:
        fields["comment"] = description
    try:
        with db.atomic():
            new_version = placcess.bump_version(pl, expected, request.webuser)
            if fields:
                Playlist.update(**fields).where(Playlist.id == pl.id).execute()
    except placcess.VersionConflict as exc:
        return _playlist_conflict(exc.playlist, "content")
    return jsonify({"ok": True, "version": new_version})


@webapi.route("/playlist/<playlist_id>", methods=["DELETE"])
@login_required
def delete_playlist(playlist_id):
    """Delete a playlist (owner/admin). Terminal: no version precondition.

    There is no lost update to prevent — the row is gone — and a 409 after a
    confirmation dialog would be hostile. Member rows and share links go with
    it (the schema's ON DELETE CASCADE).
    """
    pl, _role, error = _playlist_guard(playlist_id, "manage")
    if error:
        return error
    pl.delete_instance(recursive=True)
    return jsonify({"ok": True})


@webapi.route("/playlist/<playlist_id>/tracks", methods=["POST"])
@login_required
def add_playlist_tracks(playlist_id):
    """Append tracks. Owner, editor or admin.

    Append-only, so it is COMMUTATIVE: two clients that both send no version
    append safely under the row lock; the SPA still sends its version, and a
    mismatch is a 409 it may offer to retry (nothing was written).
    """
    pl, _role, error = _playlist_guard(playlist_id, "edit")
    if error:
        return error
    data, error = _playlist_body(PLAYLIST_BODY_TRACKS)
    if error:
        return error
    expected, error = _playlist_expected(data, "version")
    if error:
        return error
    refs, error = _playlist_refs(data)
    if error:
        return error
    if not refs:
        return jsonify({"error": "no tracks"}), 400
    provider = _provider()
    tracks = _addable_tracks(provider, refs)
    if not tracks:
        return jsonify({"error": "no tracks"}), 400
    try:
        with db.atomic():
            new_version = placcess.bump_version(pl, expected, request.webuser)
            pl.add_many(tracks)
            nb_tracks = _visible_playlist_count(pl, request.webuser)
    except placcess.VersionConflict as exc:
        return _playlist_conflict(exc.playlist, "content")
    from ..deezer import backfill

    backfill.archive_tracks(
        current_app._get_current_object(), tracks, event="on_playlist_add"
    )
    return jsonify(
        {
            "ok": True,
            "added": len(tracks),
            "version": new_version,
            "nb_tracks": nb_tracks,
        }
    )


@webapi.route("/playlist/<playlist_id>/tracks", methods=["DELETE"])
@login_required
def remove_playlist_tracks(playlist_id):
    """Remove rows by the caller's VISIBLE positions, or by track ids.

    An index is a position in the list this person sees, mapped to the stored
    ``playlist_track`` rows here; for an owner/admin (every row visible) it is
    byte-identical to the old behaviour, and an editor's index can never land
    on somebody else's private upload.
    """
    pl, _role, error = _playlist_guard(playlist_id, "edit")
    if error:
        return error
    data, error = _playlist_body(PLAYLIST_BODY_TRACKS, allow_empty=True)
    if error:
        return error
    expected, error = _playlist_expected(data, "version")
    if error:
        return error
    visible = _visible_playlist_rows(pl, request.webuser)
    if "indexes" in data:
        indexes, error = _playlist_indexes(data)
        if error:
            return error
        stored = [visible[i][1] for i in indexes if i < len(visible)]
    else:
        refs, error = _playlist_refs(data)
        if error:
            return error
        wanted = set(refs)
        stored = [
            visible[i][1]
            for i, (_ptid, _index, tid, dz) in enumerate(visible)
            if str(tid) in wanted or (dz and str(dz) in wanted)
        ]
    if not stored:
        return jsonify({"error": "no tracks"}), 400
    try:
        with db.atomic():
            new_version = placcess.bump_version(pl, expected, request.webuser)
            pl.remove_at_indexes(stored)
    except placcess.VersionConflict as exc:
        return _playlist_conflict(exc.playlist, "content")
    return jsonify({"ok": True, "removed": len(stored), "version": new_version})


@webapi.route("/playlist/<playlist_id>/order", methods=["PUT"])
@login_required
def reorder_playlist(playlist_id):
    """Replace the order with the caller's visible order, hidden rows anchored.

    The client's list is resolved with occurrence counts (duplicates are kept),
    then written as one DELETE plus chunked INSERTs inside the same transaction
    as the version CAS. Measured on 4 000 rows: 19 statements in all — the
    session, the resolve, the visible-row read, the CAS, the DELETE and 14
    INSERTs of 300 rows — never one UPDATE per row.
    """
    pl, _role, error = _playlist_guard(playlist_id, "edit")
    if error:
        return error
    data, error = _playlist_body(PLAYLIST_BODY_TRACKS)
    if error:
        return error
    expected, error = _playlist_expected(data, "version")
    if error:
        return error
    order, error = _playlist_refs(data, max_refs=PLAYLIST_MAX_ORDER)
    if error:
        return error
    if not order:
        return jsonify({"error": "no order"}), 400
    rows = _reordered_rows(pl, request.webuser, order)
    fields = (PlaylistTrack.playlist, PlaylistTrack.track, PlaylistTrack.index)
    try:
        with db.atomic():
            new_version = placcess.bump_version(pl, expected, request.webuser)
            PlaylistTrack.delete().where(PlaylistTrack.playlist == pl.id).execute()
            # Bulk insert in chunks: one statement instead of one INSERT per
            # track (a 500-track reorder was 500 round-trips on Postgres/MySQL).
            for i in range(0, len(rows), 300):
                PlaylistTrack.insert_many(
                    rows[i : i + 300], fields=fields
                ).execute()
    except placcess.VersionConflict as exc:
        return _playlist_conflict(exc.playlist, "content")
    return jsonify({"ok": True, "version": new_version})


# Most tracks one /api/download call may queue for background archiving.
_DOWNLOAD_BATCH_MAX = 2000


@webapi.route("/download", methods=["POST"])
@login_required
def download():
    """Pre-archive a set of tracks now (don't wait for playback)."""
    provider, err = _need_provider()
    if err:
        return err
    raw = (request.get_json(silent=True) or {}).get("ids") or []
    if not isinstance(raw, list):
        return jsonify({"error": "no ids"}), 400
    # De-duplicate (order-preserving) and cap the batch. The queue behind this is
    # unbounded and each entry means a full FLAC download, so one POST carrying
    # a hundred thousand ids would pin the archiver for days and fill the disk —
    # available to any logged-in account. A whole playlist still fits in one
    # request; anything larger is a client bug or an attack.
    from . import refs

    texts = [str(x) for x in raw[: _DOWNLOAD_BATCH_MAX * 2]]
    # A row UUID of a Deezer track names that track: one IN for all of them.
    guids = [x for x in texts if not _valid_id(x)]
    mapped = {}
    if guids:
        mapped = {
            x: str(row.deezer_id)
            for x, row in zip(guids, refs.resolve_tracks(guids))
            if row is not None and row.deezer_id
        }
    seen, ids = set(), []
    for x in texts:
        x = mapped.get(x, x)
        if _valid_id(x) and x not in seen:
            seen.add(x)
            ids.append(x)
            if len(ids) >= _DOWNLOAD_BATCH_MAX:
                break
    if not ids:
        return jsonify({"error": "no ids"}), 400
    pf = getattr(current_app, "deezer_prefetch", None)
    if pf is None:
        return jsonify({"error": "downloader unavailable"}), 503
    # WHY this was asked for, not how urgent the client thinks it is: the track
    # the player is about to need ("prefetch") goes ahead of the two thousand a
    # "download this playlist" (the default) queued behind it. An unknown or
    # missing reason is treated as bulk, so nothing can claim the front by
    # accident.
    from ..deezer.workload import priority_from_name

    reason = (request.get_json(silent=True) or {}).get("why")
    queued = pf.download_ids(ids, priority=priority_from_name(reason))
    if queued < len(ids):
        # The archive queue is bounded (see prefetch.py): tell the client how
        # much actually got in rather than pretending the whole batch is coming.
        return jsonify(
            {"ok": True, "queued": queued, "dropped": len(ids) - queued, "busy": True}
        ), 202
    return jsonify({"ok": True, "queued": queued})


@webapi.route("/download/status")
@login_required
def download_status():
    pf = getattr(current_app, "deezer_prefetch", None)
    return jsonify({"pending": pf.download_pending if pf else 0})


# A manual "refresh from Deezer" — the same job the auto-sync scheduler runs,
# triggered on demand. Admin-only (it touches the shared account) and run in a
# background thread so the request returns immediately; at most one at a time.
_sync_thread: threading.Thread | None = None
_sync_lock = threading.Lock()


@webapi.route("/sync", methods=["POST"])
@login_required
@admin_required
def trigger_sync():
    provider, err = _need_provider()
    if err:
        return err
    if not current_app.config["DEEZER"].get("sync_user"):
        return jsonify({"error": "no sync user configured"}), 503
    if _dz_live() is None:
        # The sync would postpone itself and the thread would exit, leaving the
        # UI to report a run that never happened. Say what is actually wrong.
        return _deezer_unreachable()

    from ..deezer import scheduler

    global _sync_thread
    with _sync_lock:
        # Already running (our own manual thread, OR the scheduled sync): don't
        # start a second one — _run_sync shares scheduler._sync_lock, so a
        # duplicate would just no-op, but reporting it cleanly avoids the spinner
        # thinking a fresh run started.
        if (_sync_thread is not None and _sync_thread.is_alive()) or scheduler.is_syncing():
            return jsonify({"ok": True, "running": True})
        app = current_app._get_current_object()
        _sync_thread = threading.Thread(
            target=scheduler._run_sync, args=(app,), name="deezer-sync-manual", daemon=True
        )
        _sync_thread.start()
    return jsonify({"ok": True, "running": True})


@webapi.route("/sync/status")
@login_required
@admin_required
def sync_status():
    from ..deezer import scheduler

    running = (_sync_thread is not None and _sync_thread.is_alive()) or scheduler.is_syncing()
    return jsonify({"running": running})


@webapi.route("/upload", methods=["POST"])
@login_required
def upload():
    """Upload audio files (any format) into the archive. They're imported on the
    spot as local library tracks — searchable, playlistable, streamable — so it's
    a one-click way to add your own music alongside Deezer.

    Non-admin users are capped at a per-user byte quota (admins are unlimited):
    files that would push them over the limit are skipped, not saved."""
    archive_dir = current_app.config["DEEZER"].get("archive_dir")
    if not archive_dir:
        return jsonify({"error": "archive directory not configured"}), 503
    files = request.files.getlist("files")
    if not files:
        return jsonify({"error": "no files"}), 400

    from ..deezer import library, local

    user = request.webuser
    # Resolve the quota once. 0 means unlimited (admins always, or a disabled
    # cap); otherwise seed the running total from what the user already has on
    # disk so this batch is checked against their real usage.
    quota_bytes, used = 0, 0
    if not _is_admin():
        gb = _quota_gb()
        if gb > 0:
            quota_bytes = int(gb * 1024**3)
            used = _user_upload_usage(archive_dir, user)

    root = library.get_root_folder(archive_dir)
    dest_dir = _user_upload_dir(archive_dir, user)
    os.makedirs(dest_dir, exist_ok=True)

    imported, skipped = [], []
    quota_hit = False
    for f in files:
        name, ext = _safe_upload_name(f.filename)
        if not name or ext not in local.AUDIO_EXTS:
            skipped.append(f.filename)
            continue
        # Refuse anything that would exceed the quota BEFORE writing it to disk.
        if quota_bytes:
            size = _stream_size(f.stream)
            if used + size > quota_bytes:
                quota_hit = True
                skipped.append(f.filename)
                continue
        base, e = os.path.splitext(os.path.join(dest_dir, name))
        dest, n = base + e, 0
        while os.path.exists(dest):
            n += 1
            dest = f"{base} ({n}){e}"
        # Belt and braces: _safe_upload_name already strips separators and
        # leading dots, so this can only fail if that ever regresses.
        if os.path.dirname(os.path.abspath(dest)) != os.path.abspath(dest_dir):
            skipped.append(f.filename)
            continue
        f.save(dest)
        try:
            # Tagged with its uploader: uploads are private to their owner
            # (see Track.visible_clause), unlike scanned library files.
            track = local.import_local_file(dest, root, owner=user)
        except Exception:
            logger.warning("Upload import failed for %s", name, exc_info=True)
            track = None
        if track is not None:
            try:
                used += os.path.getsize(dest)
            except OSError:
                pass
            # Measured now, like an archive is: its tempo and its genre (the
            # one its tags name wins, analysis.file_genre) are then the
            # server's answer by the time it is first played. Never blocks.
            try:
                from ..deezer import analysis as analysis_mod

                analysis_mod.queue_analysis(track, None)
            except Exception:
                logger.debug("upload: could not queue the analysis", exc_info=True)
            try:
                from ..enrich import queue as enrich_queue

                enrich_queue.queue_track(track.id)
            except Exception:
                logger.debug("upload: could not queue the enrichment", exc_info=True)
            imported.append(_local_track(track))
        else:
            try:
                os.remove(dest)  # unreadable audio -> don't keep it around
            except OSError:
                pass
            skipped.append(f.filename)

    resp = {"imported": imported, "skipped": skipped, "count": len(imported)}
    if quota_bytes:
        resp["quota"] = quota_bytes
        resp["used"] = used
        resp["quota_exceeded"] = quota_hit
    return jsonify(resp)


@webapi.route("/local/<track_id>", methods=["DELETE"])
@login_required
def delete_local(track_id):
    """Delete one of your own uploads — row and file.

    There was no way to remove an upload at all, so a mistaken (or unwanted)
    file stayed on the server, counted against the quota, forever. Only the
    owner (or an admin) can delete, and only uploaded files: shared-library
    tracks and Deezer rows are refused.
    """
    try:
        track = Track[uuid.UUID(str(track_id))]
    except (ValueError, Track.DoesNotExist):
        return jsonify({"error": "not found"}), 404
    if track.owner_id is None or track.deezer_id is not None:
        return jsonify({"error": "not an upload"}), 403
    if not _may_access_track(track):
        return jsonify({"error": "not found"}), 404

    path = track.path
    track.delete_instance(recursive=True)
    try:
        os.remove(path)
    except OSError:
        logger.info("Upload file already gone: %s", path)
    # The facts sidecar written beside it at import goes with it.
    try:
        os.remove(os.path.splitext(path)[0] + ".json")
    except OSError:
        pass
    return jsonify({"ok": True})


@webapi.route("/upload/usage")
@login_required
def upload_usage():
    """This user's upload usage and their cap (0 quota = unlimited)."""
    archive_dir = current_app.config["DEEZER"].get("archive_dir")
    admin = _is_admin()
    gb = _quota_gb()
    quota_bytes = int(gb * 1024**3) if (gb > 0 and not admin) else 0
    used = _user_upload_usage(archive_dir, request.webuser) if archive_dir else 0
    return jsonify(
        {
            "used": used,
            "quota": quota_bytes,
            "unlimited": admin or quota_bytes == 0,
        }
    )


# -- Deezer credential (ARL) ------------------------------------------------
# The ARL is a full-account credential, so it is NEVER sent back to the client —
# not even to the admin who set it. The UI gets "is one configured", where it
# came from and a four-character tail to recognise it by, which is everything it
# needs to render the card.


def _arl_hint(arl) -> "str | None":
    arl = (arl or "").strip()
    return f"…{arl[-4:]}" if len(arl) >= 8 else ("…" if arl else None)


def _deezer_settings() -> dict:
    from ..deezer import stored_arl

    provider = _provider()
    override = stored_arl()
    # What is actually in force: the stored override, else the configured value,
    # else whatever the running provider was built with.
    configured = (
        override
        or (current_app.config["DEEZER"].get("arl") or "")
        or (getattr(provider, "arl", "") or "")
    )
    return {
        "arl_set": bool(configured),
        # Which one is in force, so the admin knows whether the compose/env value
        # is being overridden.
        "arl_source": "database" if override else ("config" if configured else None),
        "arl_hint": _arl_hint(configured),
        "enabled": provider is not None,
        "archive_dir_set": bool(current_app.config["DEEZER"].get("archive_dir")),
    }


@webapi.route("/settings")
@login_required
@admin_required
def get_settings():
    """Admin-only server settings (upload quota + the Deezer credential state)."""
    return jsonify({"upload_quota_gb": _quota_gb(), "deezer": _deezer_settings()})


@webapi.route("/settings", methods=["POST"])
@login_required
@admin_required
def set_settings():
    """Persist admin-editable server settings (upload quota, Deezer ARL)."""
    from ..deezer import DeezerProvider, store_arl, valid_arl
    from ..web import setup_deezer

    data = request.get_json(silent=True) or {}
    if "upload_quota_gb" in data:
        try:
            v = float(data["upload_quota_gb"])
        except (TypeError, ValueError):
            return jsonify({"error": "invalid upload_quota_gb"}), 400
        if v < 0:
            return jsonify({"error": "invalid upload_quota_gb"}), 400
        _set_quota_gb(v)

    if "deezer_arl" in data:
        arl = data.get("deezer_arl")
        arl = arl.strip() if isinstance(arl, str) else ""
        if not arl:
            # Explicitly clearing the override: fall back to the configured one.
            store_arl(None)
            configured = current_app.config["DEEZER"].get("arl")
            provider = _provider()
            if provider is not None and configured:
                provider.set_arl(configured)
            else:
                setup_deezer(current_app._get_current_object())
            logger.info("Deezer ARL override cleared by %s", request.webuser.name)
        else:
            if not valid_arl(arl):
                return jsonify({"error": "ARL invalide (format inattendu)"}), 400
            if not current_app.config["DEEZER"].get("archive_dir"):
                return (
                    jsonify({"error": "archive_dir n'est pas configuré sur le serveur"}),
                    400,
                )
            # Try it BEFORE storing it: saving a dead credential would knock the
            # whole proxy out until someone noticed.
            probe = DeezerProvider(arl, current_app.config["DEEZER"]["archive_dir"])
            status = probe.check_login(force=True)
            if not status["ok"]:
                message = (
                    "Deezer a refusé cet ARL (expiré ou incorrect)"
                    if status["reason"] == "arl"
                    else "Deezer est injoignable, réessayez plus tard"
                )
                return jsonify({"error": message, "reason": status["reason"]}), 400
            store_arl(arl)
            # Whoever saves the server's ARL holds the account (plan §15).
            access.ensure_server_account(owner=request.webuser)
            provider = _provider()
            if provider is not None:
                # Swap the credential in place: the prefetcher and the sync
                # thread hold this object, and replacing it would strand their
                # worker threads on the old one.
                provider.set_arl(arl)
            else:
                # Nothing was running (no ARL at boot): build it all now, so the
                # proxy comes to life without a restart.
                setup_deezer(current_app._get_current_object())
            logger.info(
                "Deezer ARL updated by %s (account %s)",
                request.webuser.name,
                status.get("account"),
            )

    return jsonify(
        {"ok": True, "upload_quota_gb": _quota_gb(), "deezer": _deezer_settings()}
    )


# -- build identity ---------------------------------------------------------
# Deliberately open (no login): the SPA checks for a new build before anyone has
# logged in, and this only says which bundle the server is serving. Never
# cached, or a stale copy would keep the app pinned to an old build forever.
@webapi.route("/version")
def app_version():
    # Named app_version, not version: a view called `version` would shadow the
    # sibling module of the same name in this package's namespace.
    from .version import android_release, spa_build

    spa = spa_build()
    response = jsonify(
        {
            "build": spa["build"],
            "version": spa["version"],
            "android": android_release(current_app.config["WEBAPP"]),
        }
    )
    response.headers["Cache-Control"] = "no-store"
    return response


# Deezer health, polled by the SPA. Available to every logged-in user (a guest
# needs to know why the catalogue went quiet) but only the admin is told to go
# fix the credential — nobody else can.
@webapi.route("/deezer/status")
@login_required
def deezer_status():
    provider = _provider()
    if provider is None:
        return jsonify(
            {
                "ok": False,
                "reason": "disabled",
                "admin": _is_admin(),
                "message": "Le proxy Deezer est désactivé sur ce serveur.",
            }
        )
    force = request.args.get("force") == "1" and _is_admin()
    status = provider.check_login(force=force)
    if status["ok"]:
        return jsonify({"ok": True, "reason": None, "account": status["account"]})
    if status["reason"] == "arl":
        message = (
            "L'identifiant Deezer (ARL) n'est plus valide. Ouvrez Réglages → Compte "
            "pour en saisir un nouveau."
            if _is_admin()
            else "La connexion au compte Deezer a expiré. Prévenez l'administrateur."
        )
    else:
        message = "Deezer est momentanément injoignable."
    # How long until the app will try Deezer again. The server already knows —
    # saying so beats a UI that looks broken with no explanation, and it keeps
    # the SPA from re-asking every few seconds during an outage.
    outage = (getattr(provider, "outage", lambda: None)() or {})
    return jsonify(
        {
            "ok": False,
            "reason": status["reason"],
            "admin": _is_admin(),
            "message": message,
            "retry_in": outage.get("retry_in", 0),
        }
    )


# Deezer told us, in so many words, that this track has no source. That is a
# verdict about the track, not a transport failure: record it, and answer 410
# Gone rather than 502 so the caller knows not to retry. The player asks
# /track/<id>/probe on a playback error and moves straight on.
def _gone(track, deezer_id):
    from .availability import mark_unavailable

    if track is not None:
        mark_unavailable(track)
    logger.info("Track %s has no playable source; marked unavailable", deezer_id)
    return jsonify({"error": "track unavailable", "unavailable": True}), 410


# Opus transcode bitrates (kbps) the web player may request: q=OPUS_320 etc.
_OPUS_BITRATES = {320, 256, 192, 128, 64}

#: Containers a seek slice can be stream-copied from, and their muxer/mimetype.
_SLICE_COPY = {
    ".mp3": ("mp3", "audio/mpeg"),
    ".ogg": ("ogg", "audio/ogg"),
    ".opus": ("ogg", "audio/ogg"),
}


def _archive_state(path):
    """``(size, writing)`` for an archive path: is the file there, and is one coming?

    Every writer of audio here publishes atomically — ``archive.ensure_archived``
    renames ``<name>.<token>.stage.part`` onto the final name, and the live,
    YouTube and podcast paths rename ``<name>.<token>.part`` — so a file found at
    the final name is COMPLETE and ``size`` is what the caller will hear. Only
    when nothing is published is the directory listed: a ``.part`` sibling then
    says an archive for that very name is being written, which is what tells a
    waiting player that the wait is a real one and it will end. A stale ``.part``
    (a killed process) can therefore never hide a finished archive.
    """
    if not path:
        return None, False
    try:
        size = os.path.getsize(path)
    except OSError:
        size = 0
    if size > 0:
        return size, False
    name = os.path.basename(path)
    try:
        with os.scandir(os.path.dirname(path) or ".") as entries:
            for entry in entries:
                if entry.name.startswith(name + ".") and entry.name.endswith(".part"):
                    return None, True
    except OSError:
        pass
    return None, False


def _start_offset():
    """``?start=<seconds>`` into the track, and the refusal for a bad value."""
    raw = request.args.get("start")
    if raw is None or raw == "":
        return None, None
    try:
        value = float(raw)
    except (TypeError, ValueError):
        value = None
    if value is None or not math.isfinite(value) or value < 0:
        return None, (jsonify({"error": "invalid start"}), 400)
    return value, None


def _clamped_offset(start, duration):
    """The offset actually applied: never past the track's own length."""
    limit = float(duration or 0)
    if limit > 0:
        return min(start, max(0.0, limit - 0.25))
    return min(start, 86400.0)


def _not_archived(path=None):
    """Nothing on disk to seek in: 409, and whether a writer is actually at work."""
    _, writing = _archive_state(path)
    resp = jsonify({"error": "not archived yet", "ready": False, "streaming": writing})
    resp.status_code = 409
    resp.headers["Retry-After"] = "2"
    return resp


def _mark_generated(resp):
    """A stream generated as it is sent: no length, no byte range, not cacheable."""
    resp.headers["Accept-Ranges"] = "none"
    resp.headers["Cache-Control"] = "no-store"
    return resp


def _transcode_key(entity, bitrate):
    """The transcode cache key ``/stream`` uses for this entity (and this quality)."""
    if isinstance(entity, PodcastEpisode):
        return f"episode-{entity.id}-opus{bitrate}.ogg"
    if getattr(entity, "youtube_id", None) and not getattr(entity, "deezer_id", None):
        return f"youtube-{entity.id}-opus{bitrate}.ogg"
    return f"deezer-{entity.id}-opus{bitrate}.ogg"


def _seek_is_generated(entity, bitrate):
    """Whether ``/stream`` answers this entity/quality with an Opus transcode."""
    if not bitrate:
        return False
    is_youtube = bool(
        getattr(entity, "youtube_id", None) and not getattr(entity, "deezer_id", None)
    )
    if not is_youtube:
        return True
    source_rate = int(getattr(entity, "bitrate", 0) or 0)
    from . import youtube

    return bool(source_rate and bitrate < source_rate - youtube.TRANSCODE_MARGIN)


def _sliceable(path, bitrate):
    """Whether ``?start=`` into this source can be answered (see ``_slice_generator``)."""
    if bitrate:
        return True
    suffix = os.path.splitext(str(path or ""))[1].lower()
    return suffix == ".flac" or suffix in _SLICE_COPY


def _slice_generator(path, start, bitrate):
    """ffmpeg from ``start`` seconds into ``path``; ``(mimetype, generator)``.

    ``-ss`` before ``-i`` is ffmpeg's input seek: it starts on the sample asked
    (on a marked FLAC, ``-ss 119`` leaves exactly 1.000 s of silence and
    ``-ss 121`` none) and costs 40 ms to the first byte where encoding from 0
    costs 71 ms. A FLAC stream copy keeps the source's STREAMINFO and would
    claim the whole track's length, so FLAC is re-encoded — lossless — while an
    MP3 or Ogg source is copied (its duration comes out right, measured). An MP4
    is left out: its muxer needs a seekable output and writes nothing to a pipe,
    and an MP4 upload is served as a file, which a byte range already seeks.
    ``(None, None)`` for a container neither can serve.
    """
    import subprocess

    if not _sliceable(path, bitrate):
        return None, None
    suffix = os.path.splitext(str(path))[1].lower()
    if bitrate:
        codec = [
            "-c:a", "libopus", "-b:a", f"{bitrate}k", "-vbr", "on",
            "-flush_packets", "1", "-page_duration", "100000",
        ]
        muxer, mimetype = "ogg", "audio/ogg"
    elif suffix == ".flac":
        codec, muxer, mimetype = ["-c:a", "flac"], "flac", "audio/flac"
    else:
        muxer, mimetype = _SLICE_COPY[suffix]
        codec = ["-c", "copy"]

    cmd = [
        "ffmpeg", "-v", "0", "-ss", f"{start:.3f}", "-i", str(path),
        "-map", "0:a:0", *codec, "-vn", "-f", muxer, "pipe:1",
    ]

    def generate():
        proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        failed = True
        try:
            while True:
                data = proc.stdout.read1(65536)
                if not data:
                    break
                yield data
            failed = proc.wait() != 0
        except GeneratorExit:
            proc.kill()
            raise
        finally:
            proc.stdout.close()
            proc.wait()
        if failed:
            logger.warning("Seek slice of %s at %.3f s failed", path, start)
            raise RuntimeError("seek slice failed")

    return mimetype, generate()


def _slice_response(path, start, bitrate):
    """A seek answered by encoding from ``start`` — never by replaying from 0."""
    mimetype, gen = _slice_generator(path, start, bitrate)
    if gen is None:
        resp = jsonify({"error": "seek with a byte range", "ranges": "bytes"})
        resp.status_code = 409
        return resp
    resp = _mark_generated(current_app.response_class(gen, mimetype=mimetype))
    resp.headers["X-NS-Start"] = f"{start:.3f}"
    return resp


def _then(first, rest):
    """``first`` (already read, or None), then the rest of the generator ``rest``.

    Not ``itertools.chain``: when the client leaves, the WSGI server closes the
    response iterable, and a chain has no ``close()`` to pass that on — the live
    stream's cleanup (drop the partial, queue the archive) would then wait for
    the garbage collector.
    """
    try:
        if first is not None:
            yield first
        yield from rest
    finally:
        rest.close()


def _opus_generator(flac_path, bitrate):
    """ffmpeg: decode the archived FLAC and (re)encode to Opus-in-Ogg.

    Raises when ffmpeg did not finish: the cache publishes what the generator
    completed, and a truncated Opus used to be cached for good — the next plays
    then served a 15 kB file that claimed to be the track.
    """
    import subprocess

    cmd = [
        "ffmpeg", "-v", "0", "-i", flac_path,
        "-map", "0:a:0", "-c:a", "libopus", "-b:a", f"{bitrate}k", "-vbr", "on",
        "-vn", "-f", "ogg", "pipe:1",
    ]
    # stderr -> /dev/null so an unread, full stderr pipe can't deadlock ffmpeg.
    proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    failed = True
    try:
        while True:
            data = proc.stdout.read(8192)
            if not data:
                break
            yield data
        failed = proc.wait() != 0
    except GeneratorExit:
        # Client disconnected mid-stream: stop ffmpeg promptly instead of
        # letting it run to completion (wasted CPU under load).
        proc.kill()
        raise
    finally:
        proc.stdout.close()
        proc.wait()
    if failed:
        logger.warning("Transcode of %s failed; nothing cached", flac_path)
        raise RuntimeError("transcode failed")


def _opus_live_generator(first, source, bitrate):
    """Opus from a source that is still downloading: the first play of a track.

    The Opus path used to archive the whole FLAC before its first byte went out
    (tens of seconds on a slow link), where the FLAC path streams as it
    downloads. Here the live, archiving stream (``archive.open_live_stream``)
    is fed straight into ffmpeg's stdin by a thread of its own while this
    generator hands ffmpeg's output to the client: the first note is the
    download's first block plus ffmpeg's start-up.

    The feeding thread owns ``source`` (a generator runs on one thread at a
    time): it consumes it to the end — which is what publishes the archive —
    or, when the listener leaves and ffmpeg is killed, its next write fails
    and it closes ``source``, which drops the partial and queues the archive
    in the background exactly as the FLAC path does.
    """
    import subprocess

    # Out as soon as it is encoded: every packet flushed to the pipe, in Ogg
    # pages of 100 ms rather than the muxer's one second. Measured at 512 KB/s,
    # the first audio page left after 0.77 s without them, 0.26 s with.
    cmd = [
        "ffmpeg", "-v", "0", "-i", "pipe:0",
        "-map", "0:a:0", "-c:a", "libopus", "-b:a", f"{bitrate}k", "-vbr", "on",
        "-vn", "-flush_packets", "1", "-page_duration", "100000",
        "-f", "ogg", "pipe:1",
    ]
    proc = subprocess.Popen(
        cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL
    )
    state = {"finished": False}

    def feed():
        # The archive is finalised on this thread (the row is refreshed, the
        # sidecars written): it needs a connection of its own.
        try:
            db.connect(reuse_if_open=True)
        except Exception:  # pragma: no cover - connection setup best-effort
            pass
        try:
            if first:
                proc.stdin.write(first)
            for chunk in source:
                proc.stdin.write(chunk)
            state["finished"] = True
        except (BrokenPipeError, ValueError, OSError):
            pass  # ffmpeg is gone: the listener left
        except Exception:
            logger.warning("Live Opus source failed", exc_info=True)
        finally:
            source.close()
            try:
                proc.stdin.close()
            except OSError:
                pass
            try:
                db.close()
            except Exception:  # pragma: no cover
                pass

    feeder = threading.Thread(target=feed, name="opus-live-feed", daemon=True)
    feeder.start()
    complete = False
    try:
        while True:
            # read1: whatever is there now. read(n) waits for n bytes, and the
            # first pages are smaller than that.
            data = proc.stdout.read1(65536)
            if not data:
                break
            yield data
        complete = proc.wait() == 0 and state["finished"]
    except GeneratorExit:
        proc.kill()
        raise
    finally:
        proc.stdout.close()
        proc.wait()
    if not complete:
        logger.warning("Live Opus transcode was cut short; nothing cached")
        raise RuntimeError("incomplete live transcode")


def _stream_episode(episode, bitrate, start=None):
    """Stream a podcast episode: archived MP3 from disk, Opus-transcoded on
    request (cached), archiving from the podcast host on first play."""
    from ..deezer import archive

    if not (episode.path and os.path.isfile(episode.path)):
        if start is not None:
            return _not_archived(episode.path)
        provider, err = _need_provider()
        if err:
            return err
        if not bitrate:
            # Stream it AS it downloads. Episodes run to tens of megabytes, so
            # archiving the whole thing before the first byte went out is why a
            # long show never started playing at all.
            pf = getattr(current_app, "deezer_prefetch", None)
            eid = str(episode.id)
            from ..deezer.workload import Priority as _P

            on_abort = (
                (lambda: pf.download_episode_ids([eid], priority=_P.PREFETCH))
                if pf
                else None
            )
            try:
                mimetype, gen = archive.open_live_episode_stream(
                    provider, episode, on_abort
                )
            except Exception:
                _log_deezer_failure("Episode live stream failed for %s", episode.id)
                return jsonify({"error": "episode unavailable"}), 502
            return _mark_generated(current_app.response_class(gen, mimetype=mimetype))
        # An Opus transcode needs the whole source on disk first.
        try:
            archive.ensure_episode_archived(provider, episode)
        except Exception:
            _log_deezer_failure("Episode fetch failed for %s", episode.id)
            return jsonify({"error": "episode unavailable"}), 502

    if start is not None:
        return _slice_response(
            episode.path, _clamped_offset(start, episode.duration), bitrate
        )
    if bitrate:
        cache = current_app.transcode_cache
        key = f"episode-{episode.id}-opus{bitrate}.ogg"
        if cache.has(key):
            return send_file(cache.get(key), mimetype="audio/ogg", conditional=True)
        return _mark_generated(
            current_app.response_class(
                cache.set_generated(key, lambda: _opus_generator(episode.path, bitrate)),
                mimetype="audio/ogg",
            )
        )
    return send_file(episode.path, mimetype=episode.mimetype, conditional=True)


def _serve_embedded_cover(track):
    """Serve a track's archived cover: the image embedded in the audio file
    (tagged at archive time), else a cover file in its folder. Cached on disk."""
    cache = current_app.cache
    key = f"localcover-{track.id}"
    if cache.has(key):
        return send_file(cache.get(key), conditional=True)

    # Embedded artwork first.
    try:
        import mediafile

        mf = mediafile.MediaFile(track.path)
        if mf.images:
            img = mf.images[0]
            return send_file(
                cache.set(key, img.data),
                mimetype=getattr(img, "mime_type", None) or "image/jpeg",
                conditional=True,
            )
    except Exception:
        pass

    # Fall back to a cover file sitting in the track's folder.
    try:
        from ..covers import find_cover_in_folder

        found = find_cover_in_folder(os.path.dirname(track.path))
        if found:
            # ``find_cover_in_folder`` answers a ``CoverFile`` (its name and its
            # score), not a path: joining the name back to the folder is what
            # turns it into the file send_file can serve.
            return send_file(os.path.join(os.path.dirname(track.path), found.name), conditional=True)
    except Exception:
        pass
    return jsonify({"error": "no cover"}), 404


@webapi.route("/localcover/<track_id>")
@login_required
def local_cover(track_id):
    """Cover art for a local (imported) track: embedded image, else folder art."""
    try:
        track = Track[uuid.UUID(str(track_id))]
    except (ValueError, Track.DoesNotExist):
        return jsonify({"error": "not found"}), 404
    if not _may_access_track(track):
        return jsonify({"error": "not found"}), 404
    return _serve_embedded_cover(track)


@webapi.route("/cover/<cid>")
@login_required
def cover(cid):
    """Cover art for ANY track — a Deezer numeric id or a local UUID — served
    same-origin. An archived track serves its embedded image; a not-yet-archived
    one proxies the art from Deezer (cached on disk). This gives the web player
    a reliable same-origin URL for the OS media-notification artwork and the
    offline cover cache, instead of depending on the client reaching the Deezer
    image CDN (flaky enough to regularly leave the notification artless)."""
    return cover_response(cid)


def _album_cover_response(cid, vetted):
    """``/cover/<album uuid>`` from the database and the disk, with no request
    to Deezer: the archived ``cover.jpg``, else the art embedded in (or beside)
    a track of the album that the caller may read, else — a Deezer album with
    no file yet — a redirect to the CDN address the row's ``cover_md5``
    spells. None when `cid` is not an album."""
    from ..deezer import library

    try:
        alb = Album[uuid.UUID(str(cid))]
    except (ValueError, Album.DoesNotExist):
        return None
    first = alb.tracks.first()
    if first is not None and not vetted and not _may_access_track(first):
        # Someone else's private upload: indistinguishable from an unknown id.
        return jsonify({"error": "not found"}), 404
    local = library.album_cover_file(alb)
    if local:
        return send_file(local, conditional=True)
    for tr in alb.tracks.where(Track.last_modification > 0).limit(20):
        if (vetted or _may_access_track(tr)) and os.path.isfile(tr.path):
            found = _serve_embedded_cover(tr)
            if not isinstance(found, tuple):  # (json, 404) when it has none
                return found
    if _real_md5(alb.cover_md5):
        return redirect(_image("cover", alb.cover_md5, 1000))
    return jsonify({"error": "not found"}), 404


def _safe_cover_path(path, provider, cache):
    """``path`` when it lives under the archive or the cover cache, else None.

    ``deezer_cover_path`` derives the file from a UUID and a model row, but the
    id comes from the URL; checking the two roots before ``send_file`` keeps a
    malformed/forged id from naming anything outside the server's own files.
    """
    if not path:
        return None
    for root in (getattr(provider, "archive_dir", None), getattr(cache, "_cache_dir", None)):
        if not root:
            continue
        try:
            real = os.path.realpath(path)
            base = os.path.realpath(root)
            rel = os.path.relpath(real, base)
            safe = safe_join(base, rel)
        except (TypeError, ValueError):
            continue
        if safe and os.path.realpath(safe) == real:
            return safe
    return None


def cover_response(cid, *, vetted=False):
    """The body of ``/cover/<cid>``, shared with the listen party.

    ``vetted`` skips the per-user access check, for a caller that has already
    established the right to this id some other way — the party serves art for
    exactly the tracks its host chose to play, to guests who have no session.
    """
    from ..deezer import archive

    if _valid_id(cid):
        track = archive.find_local_track(cid)
    else:
        try:
            track = Track[uuid.UUID(str(cid))]
        except (ValueError, Track.DoesNotExist):
            track = None
        # A UUID that isn't a track is fine (album/artist/playlist/podcast art,
        # resolved below) — but a track we may not read must not leak its cover.
        if track is not None and not vetted and not _may_access_track(track):
            return jsonify({"error": "not found"}), 404
    if track is not None and os.path.isfile(track.path):
        return _serve_embedded_cover(track)
    if track is None and not _valid_id(cid):
        resp = _album_cover_response(cid, vetted)
        if resp is not None:
            return resp
    if not _valid_id(cid) and (track is None or track.youtube_id):
        # A YouTube hit or a YouTube row not fetched yet: the search's art.
        resp = youtube.cover(track, cid)
        if resp is not None:
            return resp
        if track is not None:
            return jsonify({"error": "not found"}), 404

    provider = _provider()
    if provider is None:
        return jsonify({"error": "not found"}), 404
    cache = current_app.cache
    try:
        if not _valid_id(cid):
            # UUID without an archived Track: album/artist/playlist/podcast art.
            path = archive.deezer_cover_path(provider, cache, cid)
            if path:
                safe = _safe_cover_path(path, provider, cache)
                if safe:
                    return send_file(safe, mimetype="image/jpeg", conditional=True)
            return jsonify({"error": "not found"}), 404
        key = f"deezer-cover-{cid}"
        if cache.has(key):
            return send_file(cache.get(key), mimetype="image/jpeg", conditional=True)
        if track is not None and track.album is not None and track.album.cover_md5:
            md5 = track.album.cover_md5
        else:
            md5 = provider.get_track_info(cid).get("ALB_PICTURE")
        data = provider.fetch_cover(md5) if md5 else None
        if not data:
            return jsonify({"error": "not found"}), 404
        return send_file(cache.set(key, data), mimetype="image/jpeg", conditional=True)
    except Exception:
        logger.debug("Cover fallback failed", exc_info=True)
        return jsonify({"error": "not found"}), 404


@webapi.route("/stream/<deezer_id>/ready")
@login_required
def stream_ready(deezer_id):
    """What ``/stream/<id>?q=`` would answer right now, for a player deciding how to seek.

    ``ready`` — a finished FILE will be served: a length, ``Accept-Ranges: bytes``, byte
    ranges, so a seek anywhere lands. Otherwise the answer is generated as it is sent
    (``Accept-Ranges: none``), and the browser cannot seek it.
    ``ranges`` — ``"bytes"`` when the next answer is that file, else null.
    ``bytes`` — the size of the archive on disk, when there is one.
    ``cached`` — the Opus transcode for this ``q`` is already in the cache.
    ``offset`` — ``?start=<seconds>`` is honoured now: the source is a finished file on
    disk, so the server can begin encoding at that time in ONE request.
    ``streaming`` — nothing is published yet and an archive for this track is being
    written right now (a ``.part`` sibling of its final name), so the wait is real and
    it ends.

    A FIRST play is generated as it is sent: no length, no ranges, so the browser clamps
    a seek to what it has read and a reload would restart from 0. The player asks this
    instead of re-fetching blind. Cheap by contract: one indexed lookup, one directory
    listing and a cache test — it never starts a download or a transcode."""
    from ..deezer import archive
    from .refs import canonical

    deezer_id = canonical(deezer_id)
    quality = (request.args.get("q") or "").upper()
    bitrate = None
    if quality.startswith("OPUS_"):
        try:
            b = int(quality.split("_", 1)[1])
            bitrate = b if b in _OPUS_BITRATES else 128
        except ValueError:
            bitrate = 128
    entity = None
    if _valid_id(deezer_id):
        entity = archive.find_local_track(deezer_id)
    else:
        try:
            key = uuid.UUID(deezer_id)
        except ValueError:
            return jsonify({"error": "invalid id"}), 400
        try:
            entity = Track[key]
        except Track.DoesNotExist:
            try:
                entity = PodcastEpisode[key]
            except PodcastEpisode.DoesNotExist:
                return jsonify({"error": "invalid id"}), 400
        if not _may_access_track(entity):
            return jsonify({"error": "invalid id"}), 400

    path = getattr(entity, "path", None) if entity else None
    size, writing = _archive_state(path)
    generated = entity is not None and _seek_is_generated(entity, bitrate)
    cached = bool(
        generated and current_app.transcode_cache.has(_transcode_key(entity, bitrate))
    )
    final = size is not None
    ready = cached if generated else final
    return jsonify(
        {
            "ready": ready,
            "ranges": "bytes" if ready else None,
            "bytes": size,
            "cached": cached,
            "offset": bool(final and _sliceable(path, bitrate)),
            "streaming": writing,
        }
    )


@webapi.route("/stream/<deezer_id>")
@login_required
@_count_stream_response
def stream(deezer_id):
    """Stream a track. Always FLAC-archived; lower qualities are Opus transcodes
    of that archived master, cached (so repeat plays are instant and seekable).

    A track that's already archived — or a local (imported) file — is served
    straight from disk with no Deezer call, so downloaded music keeps playing
    even if Deezer is unreachable. Local tracks are addressed by their UUID.

    A seek: a finished file answers a byte ``Range`` (206). A source that is on disk
    but has to be encoded answers ``?start=<seconds>`` with the audio from that time
    (``X-NS-Start``, ``Accept-Ranges: none``, never cached: a slice is not the whole
    track). A source that is not on disk answers 409 + ``Retry-After`` until
    ``/stream/<id>/ready`` says ``ready`` or ``offset``.
    """
    from ..deezer import archive
    from .refs import canonical

    # A row UUID of a Deezer track is that track: same answer as its Deezer id.
    deezer_id = canonical(deezer_id)

    # Resolve the requested quality up front: it decides whether a cold track can
    # take the stream-first (live FLAC) fast path or must be fully archived first.
    quality = (request.args.get("q") or "").upper()  # e.g. OPUS_128
    bitrate = None
    if quality.startswith("OPUS_"):
        try:
            b = int(quality.split("_", 1)[1])
            bitrate = b if b in _OPUS_BITRATES else 128
        except ValueError:
            bitrate = 128
    start, refused = _start_offset()
    if refused is not None:
        return refused

    if _valid_id(deezer_id):
        track = archive.find_local_track(deezer_id)  # by Deezer id
    else:
        # A UUID -> a local/imported track, or a podcast episode. Both are served
        # from disk (episodes are archived from the podcast host on first play).
        try:
            key = uuid.UUID(deezer_id)
        except ValueError:
            return jsonify({"error": "invalid id"}), 400
        try:
            track = Track[key]
        except Track.DoesNotExist:
            # A YouTube search hit becomes its row the moment it is played.
            track = youtube.materialize(key)
            if track is None:
                try:
                    episode = PodcastEpisode[key]
                except PodcastEpisode.DoesNotExist:
                    return jsonify({"error": "invalid id"}), 400
                return _stream_episode(episode, bitrate, start)
        if not _may_access_track(track):
            # Someone else's private upload: indistinguishable from an unknown id.
            return jsonify({"error": "invalid id"}), 400
        if track.youtube_id and not track.deezer_id:
            # The fallback source: fetched once on this first play, then served
            # from disk for good — as is, unless the client asked for less.
            if not os.path.isfile(track.path):
                refused = youtube.stream_cold(track)
                if refused is not None:
                    return refused
                track = Track.get_by_id(track.id)
            if start is not None:
                return _slice_response(
                    track.path, _clamped_offset(start, track.duration), bitrate
                )
            return youtube.serve(track, bitrate)

    if track is None or not os.path.isfile(track.path):
        if not _valid_id(deezer_id):
            return jsonify({"error": "track unavailable"}), 404
        if start is not None:
            return _not_archived(getattr(track, "path", None))
        # A PREFETCH must never be what downloads a FLAC on a request thread.
        # Nobody is waiting for these bytes, and holding a thread for the minute
        # the download takes is how the person who IS waiting ends up queueing
        # behind three of them. Queue it at prefetch priority and answer now; the
        # client reads 503 as "not cached yet" and tries again later.
        if getattr(request, "ns_background", False):
            from ..deezer.workload import Priority

            pf = getattr(current_app, "deezer_prefetch", None)
            if pf is not None:
                pf.download_ids([deezer_id], priority=Priority.PREFETCH)
            resp = jsonify({"error": "not archived yet", "queued": True})
            resp.status_code = 503
            resp.headers["Retry-After"] = "30"
            return resp
        # Not archived yet — we need Deezer for the metadata and/or the audio.
        provider, err = _need_provider()
        if err:
            return err
        try:
            if track is None:
                # Metadata only (the DB row) — no audio download yet.
                track = archive.import_track(provider, deezer_id)
        except TrackUnavailable:
            return _gone(track, deezer_id)
        except Exception:
            _log_deezer_failure("Stream metadata fetch failed for %s", deezer_id)
            return jsonify({"error": "track unavailable"}), 502

        if not os.path.isfile(track.path):
            # Stream the archive AS it downloads so playback starts almost
            # instantly — as FLAC, or through ffmpeg into Opus; the archive is
            # finalized when the download completes. If the client disconnects
            # early, on_abort queues a normal background archive so it still
            # caches.
            pf = getattr(current_app, "deezer_prefetch", None)
            # They were listening to it and the connection dropped: finish
            # the archive ahead of anything bulk, not behind it.
            from ..deezer.workload import Priority as _P

            on_abort = (
                (lambda did=deezer_id: pf.download_ids([did], priority=_P.PREFETCH))
                if pf
                else None
            )
            try:
                mimetype, gen = archive.open_live_stream(provider, track, on_abort)
                # The first block is read here rather than by the WSGI
                # server — which sends the headers with it anyway — so a
                # stream refused on its first bytes (the CDN says no, it is
                # empty, it does not decrypt to audio) gets this route's 502
                # instead of dying with the response half-sent.
                first = next(gen, None)
            except TrackUnavailable:
                return _gone(track, deezer_id)
            except Exception:
                _log_deezer_failure("Live stream failed for %s", deezer_id)
                return jsonify({"error": "track unavailable"}), 502
            if not bitrate:
                return _mark_generated(
                    current_app.response_class(_then(first, gen), mimetype=mimetype)
                )
            # Opus: the same live download, transcoded as it arrives (it
            # used to wait for the whole FLAC). A complete transcode lands
            # in the cache like any other, so the next play is seekable.
            key = _transcode_key(track, bitrate)
            return _mark_generated(
                current_app.response_class(
                    current_app.transcode_cache.set_generated(
                        key, lambda: _opus_live_generator(first, gen, bitrate)
                    ),
                    mimetype="audio/ogg",
                )
            )

    if start is not None:
        return _slice_response(
            track.path, _clamped_offset(start, track.duration), bitrate
        )

    if bitrate:
        cache = current_app.transcode_cache
        key = _transcode_key(track, bitrate)
        if cache.has(key):
            return send_file(cache.get(key), mimetype="audio/ogg", conditional=True)
        return _mark_generated(
            current_app.response_class(
                cache.set_generated(key, lambda: _opus_generator(track.path, bitrate)),
                mimetype="audio/ogg",
            )
        )

    # Lossless: serve the archived FLAC (range/seek via send_file).
    return send_file(track.path, mimetype=track.mimetype, conditional=True)


# Share endpoints (waveform / file / clip) live in their own module; importing
# it registers its routes on this blueprint. Must stay at the bottom: it
# imports helpers defined above.
from . import availability  # noqa: E402,F401  isort:skip
from . import automatch  # noqa: E402,F401  isort:skip
from . import storage  # noqa: E402,F401  isort:skip
from . import share  # noqa: E402,F401  isort:skip
from . import edges  # noqa: E402,F401  isort:skip
from . import analysis  # noqa: E402,F401  isort:skip
from . import genre  # noqa: E402,F401  isort:skip
from . import neighbors  # noqa: E402,F401  isort:skip
from . import enrich as _enrich_routes  # noqa: E402,F401  isort:skip
from . import genreshare  # noqa: E402,F401  isort:skip
from . import export  # noqa: E402,F401  isort:skip
from . import credits  # noqa: E402,F401  isort:skip
from . import diskmigrate  # noqa: E402,F401  isort:skip
from . import party  # noqa: E402,F401  isort:skip
from . import remote  # noqa: E402,F401  isort:skip
from . import youtube  # noqa: E402,F401  isort:skip
from . import services  # noqa: E402,F401  isort:skip
from . import playlist_import  # noqa: E402,F401  isort:skip
from . import androidapk  # noqa: E402,F401  isort:skip
from . import discover as _discover_routes  # noqa: E402,F401  isort:skip
from . import accessapi  # noqa: E402,F401  isort:skip
from . import myaccounts  # noqa: E402,F401  isort:skip
from . import sourcesapi  # noqa: E402,F401  isort:skip
from . import playlist_share  # noqa: E402,F401  isort:skip
from . import genre_radio  # noqa: E402,F401  isort:skip
from . import history  # noqa: E402,F401  isort:skip
from . import recap  # noqa: E402,F401  isort:skip
from . import moods  # noqa: E402,F401  isort:skip
from . import playlistrecs  # noqa: E402,F401  isort:skip
from . import playlist_members  # noqa: E402,F401  isort:skip
