# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Public links to ONE track, with a player and a rich preview.

The owner of a track asks the share sheet for a link; whoever holds it — no
account, no session — gets a small public page, the cover, and an Opus stream
of that one track. The page is rendered by the server (not the SPA) because a
messaging app's preview bot does not run JavaScript, and because a shared link
must stay cheap on a slow phone: one HTML response, one cover, and **no audio
byte until the play button is pressed**.

The link is a capability, and the rules are the ones a public audio surface
needs (W3C TAG *Good Practices for Capability URLs*, OWASP's signed-URL and
token-revocation guidance):

- **The token is the capability, and the database cannot hand one out.** It is
  ``<id>.<mac>``, the mac an HMAC of the id under the app's secret key with a
  domain prefix — the same shape as ``playlist_share.py`` and ``remote.py``. The
  row names the grant; a copy of the table opens nothing, and a secret change
  kills every link.
- **One track, and nothing else.** The row names the track; no route accepts a
  media id from the visitor, so a link can never reach a sibling track.
- **Revocation is immediate.** Rows are never cached; every public answer is
  ``Cache-Control: no-store``, ``X-Robots-Tag: noindex`` and
  ``Referrer-Policy: no-referrer``. A disabled owner kills their links too.
- **The public side never asks a provider.** The audio is archived when the
  owner creates the link (their own foreground action); the public stream only
  reads the file on disk, and answers 409 if it is gone. The stream is **Opus
  only** (128 kbit/s by default) — never the archived FLAC, never an MP3 served
  straight from Deezer.
- **A page is cheap.** The HTML is a few kB, the cover is resized to 400 px,
  and the audio element has ``preload="none"``: a recipient who does not press
  play downloads zero audio bytes.
- **Rate limited per token and client IP**, because a leaked link is still a
  public endpoint.

The public page and its preview tags follow the Open Graph protocol (``og:type
music.song``, ``og:audio``, ``og:image``), oEmbed 1.0 (a ``rich`` answer with an
iframe), and the Twitter player card's ``twitter:player`` fields; the pages that
started this shape are Spotify's and SoundCloud's oEmbed/track embeds and
Bandcamp's track embeds.
"""

from __future__ import annotations

import base64
import hashlib
import hmac
import html
import logging
import os
import re
import secrets
import shutil
import string
import threading
import time
import uuid
from datetime import timedelta
from io import BytesIO
from urllib.parse import quote, urlsplit

from flask import current_app, jsonify, make_response, request, send_file

from ..db import Album, Artist, Track, TrackShare, User, now
from . import _may_access_track, login_required, webapi
from .playlist_share import _RequestLimiter

logger = logging.getLogger(__name__)

TTL_MIN = 3600  # s: the shortest link one may ask for
TTL_DEFAULT = 30 * 24 * 3600  # s: a month, the share sheet's own lifetime
TTL_MAX = 30 * 24 * 3600  # s: a month at most
TOUCH_EVERY = 60.0  # s between last_used writes for one link

_DOMAIN = b"ns-track-share:"
_TOKEN_RE = re.compile(r"\A([A-Za-z0-9_-]{16,24})\.([A-Za-z0-9_-]{24})\Z")

# The public player asks for Opus; 128 kbit/s stereo is the phone-on-a-train
# compromise (the app's own "standard, light" choice). A caller may ask for a
# lower or higher one, never a lossless source.
_BITRATES = {64, 128, 192, 320}
_DEFAULT_BITRATE = 128

# A page's own cover is small; the preview's is the 640 px one most platforms
# crop to a card. Both are generated from the archived art.
PAGE_COVER_WIDTH = 400
OG_COVER_WIDTH = 640


# -- tokens ----------------------------------------------------------------------


def _mac(share_id: str) -> str:
    key = current_app.secret_key
    if isinstance(key, str):
        key = key.encode()
    digest = hmac.new(key, _DOMAIN + share_id.encode(), hashlib.sha256).digest()
    return base64.urlsafe_b64encode(digest).rstrip(b"=").decode()[:24]


def token_for(share_id: str) -> str:
    return f"{share_id}.{_mac(share_id)}"


def _parse_token(token):
    """The share id a token names, or None when it is not one this process
    minted (malformed, or a forged mac)."""
    if not isinstance(token, str):
        return None
    m = _TOKEN_RE.match(token.strip())
    if not m:
        return None
    share_id, mac = m.groups()
    return share_id if hmac.compare_digest(mac, _mac(share_id)) else None


# -- live rows --------------------------------------------------------------------


def _alive(row) -> bool:
    if row is None or row.revoked is not None:
        return False
    return row.expires is None or row.expires > now()


def _live_row(share_id):
    """The share row if it still grants anything, else None. Deliberately not
    cached: a revocation must be dead on the very next public request."""
    row = TrackShare.get_or_none(TrackShare.id == share_id)
    if not _alive(row):
        return None
    if User.select(User.disabled).where(User.id == row.user_id).scalar():
        return None
    return row


_touched: dict = {}
_touched_lock = threading.Lock()


def _touch_used(row):
    """Best-effort ``last_used`` stamp, throttled so a busy link does not write
    once per request."""
    t = time.monotonic()
    with _touched_lock:
        if t - _touched.get(row.id, -1e9) < TOUCH_EVERY:
            return
        _touched[row.id] = t
    try:
        TrackShare.update(last_used=now()).where(TrackShare.id == row.id).execute()
    except Exception:  # a timestamp is never worth failing the request over
        logger.debug("Could not stamp last_used on %s", row.id, exc_info=True)


def _no_store(resp):
    """A public answer must be un-cacheable everywhere (revocation is immediate)
    and invisible to crawlers."""
    resp.headers["Cache-Control"] = "no-store"
    resp.headers["X-Robots-Tag"] = "noindex, nofollow"
    resp.headers["Referrer-Policy"] = "no-referrer"
    return resp


def _share_path(share_id: str) -> str:
    """The short link path, under whatever prefix the app is mounted at."""
    return f"{request.script_root.rstrip('/')}/t/{token_for(share_id)}"


def _absolute(path: str) -> str:
    return request.url_root.rstrip("/") + path


def _link_values(share_id, created, expires, last_used) -> dict:
    return {
        "id": share_id,
        "token": token_for(share_id),
        "url": _share_path(share_id),
        "created": created.isoformat() if created else None,
        "expires": expires.isoformat() if expires else None,
        "last_used": last_used.isoformat() if last_used else None,
    }


def _link_payload(row) -> dict:
    return _link_values(row.id, row.created, row.expires, row.last_used)


# -- limiters ----------------------------------------------------------------------


# A page, its data and its cover are cheap; the audio is the expensive one and
# gets the tighter window. Both are per token+client, like the playlist link.
_page_limiter = _RequestLimiter(max_requests=240, window=600)
_stream_limiter = _RequestLimiter(max_requests=120, window=600)


def _limited(limiter, share):
    allowed, retry = limiter.allow(f"{share.id}:{request.remote_addr or ''}")
    if allowed:
        return None
    out = _no_store(jsonify({"error": "too many requests"}))
    out.status_code = 429
    out.headers["Retry-After"] = str(retry)
    return out


def _clear_limiters():
    """Used by the tests: a process-wide window would otherwise leak between
    cases."""
    _page_limiter.clear()
    _stream_limiter.clear()


# -- owner side --------------------------------------------------------------------


def _owned_track(mid):
    """``(track, error)`` for the owner side; an episode is not a track here and
    a private upload of somebody else reads as not found."""
    from .share import _resolve_media

    track, episode = _resolve_media(mid)
    if episode is not None or track is None:
        return None, (jsonify({"error": "not found"}), 404)
    if not _may_access_track(track):
        return None, (jsonify({"error": "not found"}), 404)
    return track, None


def _live_for(track, user):
    return (
        TrackShare.select()
        .where(
            TrackShare.track == track,
            TrackShare.user == user,
            TrackShare.revoked.is_null(True),
        )
        .order_by(TrackShare.created.desc())
        .first()
    )


def _ttl_from_body():
    """``(ttl, error)``: absent means the sheet's own month; explicit null means
    no expiry; anything else is a bounded number of seconds."""
    data = request.get_json(silent=True)
    if data is None:
        data = {}
    if not isinstance(data, dict):
        return None, (jsonify({"error": "invalid body"}), 400)
    if "ttl" not in data:
        return TTL_DEFAULT, None
    if data["ttl"] is None:
        return None, None
    try:
        ttl = int(data["ttl"])
    except (TypeError, ValueError):
        return None, (jsonify({"error": "ttl must be a number of seconds"}), 400)
    if not TTL_MIN <= ttl <= TTL_MAX:
        return (
            None,
            (
                jsonify(
                    {"error": f"ttl must be between {TTL_MIN} and {TTL_MAX} seconds"}
                ),
                400,
            ),
        )
    return ttl, None


def _archive_for_share(track, mid):
    """Make sure the audio is on disk before a public link names it.

    The owner's own action is foreground work (Priority.USER), exactly like the
    file-share sheet's on-demand archive; nothing in the public routes ever
    calls a provider. A YouTube fallback row goes through its own one-time
    fetch. ``(track, error)``: the row is refreshed after the fetch because
    archiving may have changed its path.
    """
    if track.path and os.path.isfile(track.path):
        return track, None

    if track.deezer_id:
        from .share import _media_file

        path, meta, err = _media_file(mid)
        if err:
            return None, err
        if not (path and os.path.isfile(path)):
            return None, (jsonify({"error": "not archived"}), 409)
        try:
            track = Track[uuid.UUID(str(meta["key"]))]
        except (KeyError, ValueError, Track.DoesNotExist):
            return None, (jsonify({"error": "not found"}), 404)
        return track, None

    if track.youtube_id:
        from ..api.media import _ensure_youtube_archived

        try:
            _ensure_youtube_archived(track)
        except Exception:
            logger.warning(
                "share: YouTube archive failed for %s", track.id, exc_info=True
            )
            return None, (jsonify({"error": "not archived"}), 409)
        track = Track.get_or_none(Track.id == track.id)
        if track is not None and track.path and os.path.isfile(track.path):
            return track, None
        return None, (jsonify({"error": "not archived"}), 409)

    # A local upload whose file is gone: never invent one.
    return None, (jsonify({"error": "not archived"}), 409)


@webapi.route("/track/<mid>/share")
@login_required
def track_share(mid):
    """The caller's live public link for this track, or ``link: null``."""
    track, error = _owned_track(mid)
    if error:
        return error
    row = _live_for(track, request.webuser)
    if not _alive(row):
        return jsonify({"link": None})
    return jsonify({"link": _link_payload(row)})


@webapi.route("/track/<mid>/share", methods=["POST"])
@login_required
def track_share_create(mid):
    """Create the public link (``ttl`` seconds, 3600..30 days; absent = a month,
    null = forever), or hand back the live link that already exists — one per
    track is enough. The track is archived here, on the owner's action."""
    track, error = _owned_track(mid)
    if error:
        return error
    ttl, error = _ttl_from_body()
    if error:
        return error

    row = _live_for(track, request.webuser)
    if _alive(row):
        # The link is live but its file may have been moved or cleaned: the
        # owner's own retry re-archives it rather than handing back a page that
        # cannot play. A failing provider is the same 502/503 as a first mint.
        if not (track.path and os.path.isfile(track.path)):
            track, error = _archive_for_share(track, mid)
            if error:
                return error
        return jsonify({"link": _link_payload(row)})

    track, error = _archive_for_share(track, mid)
    if error:
        return error

    expires = now() + timedelta(seconds=ttl) if ttl else None
    row = TrackShare.create(
        id=secrets.token_urlsafe(12),
        track=track,
        user=request.webuser,
        expires=expires,
    )
    return jsonify({"link": _link_payload(row)})


@webapi.route("/track/<mid>/share", methods=["DELETE"])
@login_required
def track_share_revoke(mid):
    """Revoke every live link for this track. Dead on the next request."""
    track, error = _owned_track(mid)
    if error:
        return error
    n = (
        TrackShare.update(revoked=now())
        .where(
            TrackShare.track == track,
            TrackShare.user == request.webuser,
            TrackShare.revoked.is_null(True),
        )
        .execute()
    )
    return jsonify({"revoked": n})


@webapi.route("/track-shares")
@login_required
def track_shares_list():
    """Every live link the caller holds, newest first. Reads the rows as tuples
    (one SELECT, no per-row artist/album hydration) and never shows somebody
    else's grant."""
    rows = list(
        TrackShare.select(
            TrackShare.id,
            TrackShare.created,
            TrackShare.expires,
            TrackShare.last_used,
            Track.id.alias("track_id"),
            Track.title,
            Artist.name.alias("artist"),
            Album.name.alias("album"),
        )
        .join(Track)
        .switch(TrackShare)
        .join(Artist, on=(Track.artist == Artist.id))
        .switch(Track)
        .join(Album, on=(Track.album == Album.id))
        .where(
            TrackShare.user == request.webuser,
            TrackShare.revoked.is_null(True),
        )
        .order_by(TrackShare.created.desc())
        .tuples()
    )
    links = []
    for sid, created, expires, last_used, tid, title, artist, album in rows:
        if expires is not None and expires <= now():
            continue
        payload = _link_values(sid, created, expires, last_used)
        payload["track"] = {
            "id": str(tid),
            "title": title,
            "artist": artist,
            "album": album,
            "cover": f"{request.script_root.rstrip('/')}/api/cover/{tid}",
        }
        links.append(payload)
    return jsonify({"links": links})


@webapi.route("/track-shares/<sid>", methods=["DELETE"])
@login_required
def track_share_delete(sid):
    """Revoke one of the caller's links. Somebody else's reads as not found."""
    row = TrackShare.get_or_none(TrackShare.id == sid)
    if row is None or row.user_id != request.webuser.id:
        return jsonify({"error": "not found"}), 404
    if row.revoked is None:
        TrackShare.update(revoked=now()).where(TrackShare.id == row.id).execute()
    return jsonify({"revoked": 1})


# -- public side -------------------------------------------------------------------


def _public_share(token):
    share_id = _parse_token(token)
    return _live_row(share_id) if share_id else None


def _public_track(share):
    return Track.get_or_none(Track.id == share.track_id)


def _cover_original(track):
    """The archived cover's bytes: the cache first, then the embedded image,
    then a cover file in the track's folder. Never a provider call."""
    cache = current_app.cache
    key = f"localcover-{track.id}"
    if cache.has(key):
        try:
            with open(cache.get(key), "rb") as fh:
                return fh.read()
        except OSError:
            pass
    try:
        import mediafile

        mf = mediafile.MediaFile(track.path)
        if mf.images:
            data = mf.images[0].data
            cache.set(key, data)
            return data
    except Exception:  # nosec B110 - "no cover" is the 404 below
        pass
    try:
        from ..covers import find_cover_in_folder

        found = find_cover_in_folder(os.path.dirname(track.path))
        if found:
            with open(os.path.join(os.path.dirname(track.path), found.name), "rb") as fh:
                data = fh.read()
            cache.set(key, data)
            return data
    except Exception:  # nosec B110 - "no cover" is the 404 below
        pass
    try:
        from ..deezer import library

        found = library.album_cover_file(track.album) if track.album else None
        if found and os.path.isfile(found):
            with open(found, "rb") as fh:
                data = fh.read()
            cache.set(key, data)
            return data
    except Exception:  # nosec B110 - "no cover" is the 404 below
        pass
    return None


def _cover_available(track):
    """Whether the page can offer art, without a query or a read of the file.

    ``has_art`` is stamped when the archive writes the cover (embedded art) or
    the folder's ``cover.jpg``; the cache carries the bytes for an already
    served track. A cover that is only in the album folder and absent from both
    is not worth an extra SELECT on every public page request: the page falls
    back to its groove, and the cover route still serves the file if a client
    asks for it.
    """
    if current_app.cache.has(f"localcover-{track.id}"):
        return True
    return bool(getattr(track, "has_art", False))


def _cover_path(track, width):
    """A resized JPEG of the archived cover, cached by (track, width)."""
    width = max(64, min(int(width), 1200))
    cache = current_app.cache
    key = f"trackshare-cover-{track.id}-{width}"
    if cache.has(key):
        return cache.get(key)
    data = _cover_original(track)
    if not data:
        return None
    try:
        from PIL import Image

        image = Image.open(BytesIO(data))
        image = image.convert("RGB")
        if image.width > width:
            image.thumbnail((width, width), Image.LANCZOS)
        out = BytesIO()
        image.save(out, "JPEG", quality=82, optimize=True)
        return cache.set(key, out.getvalue())
    except Exception:
        logger.warning("share: cover resize failed for %s", track.id, exc_info=True)
        return None


def _requested_bitrate():
    raw = (request.args.get("q") or "").upper()
    if raw.startswith("OPUS_"):
        try:
            value = int(raw[5:])
        except ValueError:
            value = _DEFAULT_BITRATE
        return value if value in _BITRATES else _DEFAULT_BITRATE
    return _DEFAULT_BITRATE


@webapi.route("/shared/track/<token>/cover")
def shared_track_cover(token):
    """The track's own cover, resized for the page or the preview. No id is
    accepted from the visitor: the token names the one track."""
    share = _public_share(token)
    if share is None:
        return _no_store(jsonify({"error": "not found"})), 404
    limited = _limited(_page_limiter, share)
    if limited is not None:
        return limited
    track = _public_track(share)
    if track is None:
        return _no_store(jsonify({"error": "not found"})), 404
    try:
        width = int(request.args.get("w") or PAGE_COVER_WIDTH)
    except (TypeError, ValueError):
        width = PAGE_COVER_WIDTH
    path = _cover_path(track, width)
    if path is None:
        return _no_store(jsonify({"error": "not found"})), 404
    _touch_used(share)
    resp = send_file(path, mimetype="image/jpeg", conditional=True)
    resp.headers["Cache-Control"] = "private, max-age=3600"
    resp.headers["X-Robots-Tag"] = "noindex, nofollow"
    resp.headers["Referrer-Policy"] = "no-referrer"
    return resp


@webapi.route("/shared/track/<token>/stream")
def shared_track_stream(token):
    """The archived audio, transcoded to Opus and nothing else: never the FLAC,
    never an MP3 straight from Deezer, never a provider call. A cached transcode
    is a real file (byte ranges, seeking); a cold one is generated as it is sent."""
    share = _public_share(token)
    if share is None:
        return _no_store(jsonify({"error": "not found"})), 404
    allowed, retry = _stream_limiter.allow(f"{share.id}:{request.remote_addr or ''}")
    if not allowed:
        resp = _no_store(jsonify({"error": "too many requests"}))
        resp.status_code = 429
        resp.headers["Retry-After"] = str(retry)
        return resp
    track = _public_track(share)
    if track is None:
        return _no_store(jsonify({"error": "not found"})), 404
    if not (track.path and os.path.isfile(track.path)):
        resp = _no_store(jsonify({"error": "not ready"}))
        resp.status_code = 409
        return resp
    if not shutil.which("ffmpeg"):
        resp = _no_store(jsonify({"error": "transcoder unavailable"}))
        resp.status_code = 503
        return resp

    from . import _mark_generated, _opus_generator, _transcode_key

    bitrate = _requested_bitrate()
    cache = current_app.transcode_cache
    key = _transcode_key(track, bitrate)
    _touch_used(share)
    if cache.has(key):
        resp = send_file(cache.get(key), mimetype="audio/ogg", conditional=True)
        resp.headers["Cache-Control"] = "no-store"
        resp.headers["X-Robots-Tag"] = "noindex, nofollow"
        return resp
    resp = current_app.response_class(
        cache.set_generated(key, lambda: _opus_generator(track.path, bitrate)),
        mimetype="audio/ogg",
    )
    _mark_generated(resp)
    resp.headers["X-Robots-Tag"] = "noindex, nofollow"
    return resp


# -- oEmbed ------------------------------------------------------------------------


_OEMBED_IFRAME = string.Template(
    "<iframe width=\"$width\" height=\"$height\" src=\"$src\""
    " title=\"$title\" loading=\"lazy\" frameborder=\"0\""
    " allow=\"autoplay; encrypted-media\"></iframe>"
)


def _token_from_url(raw):
    """The share token a submitted oEmbed URL names, or None.

    Only OUR page shape is accepted (``…/t/<token>`` or ``…/app/t/<token>``),
    with no query or fragment: the endpoint must never become a fetcher for an
    arbitrary URL.
    """
    try:
        parts = urlsplit(raw)
    except (TypeError, ValueError):
        return None
    if parts.scheme not in ("http", "https") and parts.scheme:
        return None
    if parts.query or parts.fragment:
        return None
    path = parts.path.rstrip("/")
    marker = "/t/"
    at = path.rfind(marker)
    if at < 0:
        return None
    return path[at + len(marker) :]


@webapi.route("/oembed")
def oembed():
    """oEmbed 1.0 discovery endpoint for a live track page.

    A consumer posts the page URL; the answer is a ``rich`` embed pointing at
    the compact player. The URL is parsed, never fetched."""
    if (request.args.get("format") or "json") != "json":
        return jsonify({"error": "unsupported format"}), 501
    token = _token_from_url(request.args.get("url") or "")
    share = _public_share(token) if token else None
    if share is None:
        return _no_store(jsonify({"error": "not found"})), 404
    track = _public_track(share)
    if track is None:
        return _no_store(jsonify({"error": "not found"})), 404

    page_url = _absolute(_share_path(share.id))
    embed_url = page_url + "?embed=1"
    title = _track_title(track)
    artist = _artist_name(track)
    try:
        width = max(280, min(int(request.args.get("maxwidth") or 480), 960))
        height = max(120, min(int(request.args.get("maxheight") or 180), 320))
    except (TypeError, ValueError):
        width, height = 480, 180
    payload = {
        "version": "1.0",
        "type": "rich",
        "provider_name": "Neezer",
        "provider_url": request.url_root.rstrip("/"),
        "title": title,
        "author_name": artist,
        "width": width,
        "height": height,
        "html": _render(
            _OEMBED_IFRAME, width=width, height=height, src=embed_url, title=title
        ),
    }
    if _cover_available(track):
        cover = _absolute(
            f"{request.script_root.rstrip('/')}/api/shared/track/{token_for(share.id)}/cover"
            f"?w={OG_COVER_WIDTH}"
        )
        payload["thumbnail_url"] = cover
        payload["thumbnail_width"] = OG_COVER_WIDTH
        payload["thumbnail_height"] = OG_COVER_WIDTH
    resp = _no_store(jsonify(payload))
    resp.headers["Access-Control-Allow-Origin"] = "*"
    return resp


# -- the page ----------------------------------------------------------------------


def _track_title(track):
    return _CONTROL_CHARS.sub("", track.title or "Titre").strip()


def _artist_name(track):
    try:
        return _CONTROL_CHARS.sub("", track.artist.name or "").strip()
    except Exception:
        return ""


def _album_name(track):
    try:
        return _CONTROL_CHARS.sub("", track.album.name or "").strip()
    except Exception:
        return ""


def _mmss(seconds):
    try:
        seconds = int(seconds or 0)
    except (TypeError, ValueError):
        seconds = 0
    m, s = divmod(seconds, 60)
    h, m = divmod(m, 60)
    return f"{h}:{m:02d}:{s:02d}" if h else f"{m}:{s:02d}"


#: The C0 controls and DEL. A provider row may carry them; a page, a meta
#: attribute or a tag must not, so every displayed name goes through this.
_CONTROL_CHARS = re.compile(r"[\x00-\x1f\x7f]")


class _Markup(str):
    """A fragment THIS MODULE built and already escaped.

    ``_render`` escapes every value it is handed, and a ``_Markup`` is the only
    value it inserts as is. Because ``_render`` is the only place one is made,
    the escaping rule of this file is one sentence: a value reaches the page
    escaped because it went through ``_render``, never because a call site
    remembered to call ``html.escape``.
    """


def _render(template, /, **values):
    """Fill ``template`` once, escaping every value.

    ``string.Template.substitute`` inserts each named value exactly once — so a
    title holding ``$home`` stays a title — and each one is HTML-escaped here,
    quotes included, unless it is markup ``_render`` itself produced.
    """
    return _Markup(
        template.substitute(
            {
                name: value
                if isinstance(value, _Markup)
                else html.escape(str(value), quote=True)
                for name, value in values.items()
            }
        )
    )


# One page for a live link and one for a dead one, each filled by ``_render``:
# a value reaches the markup ONCE, escaped at that boundary, so a
# client-chosen title can never inject markup. No inline script is emitted
# without the response's own nonce, and no third-party resource is allowed by
# the CSP.
_PAGE = string.Template(
    "<!doctype html><html lang=fr><head><meta charset=utf-8>"
    "<meta name=viewport content='width=device-width,initial-scale=1,viewport-fit=cover'>"
    "<title>$page_title</title>"
    "<meta name=description content='$desc'>"
    "<link rel=canonical href='$page_url'>"
    "<meta name=theme-color content='#0b0a0e'>"
    "<meta name=robots content='noindex,nofollow'>"
    "<meta property='og:type' content='music.song'>"
    "<meta property='og:title' content='$og_title'>"
    "<meta property='og:description' content='$desc'>"
    "<meta property='og:url' content='$page_url'>"
    "<meta property='og:site_name' content='Neezer'>"
    "<meta property='og:locale' content='fr_FR'>"
    "$image_tags"
    "<meta property='music:duration' content='$duration_s'>"
    "$audio_tags"
    "<meta name='twitter:card' content='summary_large_image'>"
    "<meta name='twitter:title' content='$og_title'>"
    "<meta name='twitter:description' content='$desc'>"
    "$twitter_image"
    "<meta name='twitter:player' content='$embed_url'>"
    "<meta name='twitter:player:width' content='480'>"
    "<meta name='twitter:player:height' content='180'>"
    "<link rel=alternate type='application/json+oembed' href='$oembed_url'>"
    "<style>$style</style>"
    "</head><body class='$body_class'>"
    "$background"
    "<main class=card>"
    "<header class=brand><span class=disc></span><span>Neezer</span>"
    "<span class=tag>lien privé</span></header>"
    "$body"
    "</main>"
    "<script nonce='$nonce'>$script</script>"
    "</body></html>"
)

_DEAD_PAGE = string.Template(
    "<!doctype html><html lang=fr><head><meta charset=utf-8>"
    "<meta name=viewport content='width=device-width,initial-scale=1'>"
    "<title>Lien expiré · Neezer</title>"
    "<meta name=robots content='noindex,nofollow'>"
    "<meta name=theme-color content='#0b0a0e'>"
    "<meta property='og:title' content='Lien expiré · Neezer'>"
    "<meta property='og:description' content='Ce lien de partage n’est plus valide.'>"
    "<style>$style</style>"
    "</head><body class='dead'>"
    "<main class=card>"
    "<header class=brand><span class=disc></span><span>Neezer</span></header>"
    "<div class=deadmsg>"
    "<span class=deadmark></span>"
    "<h1>Ce lien n’est plus valide.</h1>"
    "<p>Il a été révoqué ou a expiré. Demandez à la personne qui vous l’a envoyé de le recréer.</p>"
    "</div>"
    "</main></body></html>"
)

_STYLE = """
:root{--bg:#0b0a0e;--s1:#141218;--s2:#1e1b24;--ln:rgba(255,255,255,.12);
--tx:#f4f1f7;--dim:#a8a2b3;--ac:#d2ff3c;--onac:#14130f;--r:22px}
*{box-sizing:border-box}
html,body{min-height:100%}
body{margin:0;background:var(--bg);color:var(--tx);
font:16px/1.45 ui-sans-serif,system-ui,-apple-system,"Segoe UI",Roboto,sans-serif;
display:grid;place-items:center;padding:24px 18px;min-height:100dvh}
.bg{position:fixed;inset:-40px;z-index:0;background-position:center;
background-size:cover;filter:blur(44px) saturate(1.25);opacity:.32;pointer-events:none}
.bg:after{content:"";position:absolute;inset:0;background:
radial-gradient(circle at 50% 30%,transparent,rgba(11,10,14,.55) 55%,var(--bg) 100%)}
body:not(.hasart) .bg{background:radial-gradient(circle at 30% 20%,#2a2433,#0b0a0e 70%)}
.card{position:relative;z-index:1;width:min(680px,100%);background:
color-mix(in srgb,var(--s1) 88%,transparent);border:1px solid var(--ln);
border-radius:var(--r);padding:22px 22px 26px;
box-shadow:0 30px 80px rgba(0,0,0,.55);backdrop-filter:blur(18px) saturate(1.2)}
.brand{display:flex;align-items:center;gap:9px;font-weight:650;font-size:.9rem;
letter-spacing:.01em;color:var(--dim);margin:0 0 20px}
.disc{width:20px;height:20px;border-radius:50%;background:var(--ac);
box-shadow:inset 0 0 0 5px var(--s1),inset 0 0 0 6px var(--ac);position:relative}
.disc:after{content:"";position:absolute;inset:8px;border-radius:50%;background:var(--s1)}
.tag{margin-left:auto;font-size:.7rem;letter-spacing:.08em;text-transform:uppercase;
border:1px solid var(--ln);border-radius:999px;padding:5px 9px;color:var(--dim)}
.who{display:flex;gap:20px;align-items:center}
.art{flex:none;width:190px;height:190px;border-radius:16px;overflow:hidden;
background:linear-gradient(145deg,#2b2536,#141218);box-shadow:0 16px 40px rgba(0,0,0,.45);
display:grid;place-items:center;position:relative}
.art img{width:100%;height:100%;object-fit:cover;display:block}
.art .groove{width:72%;height:72%;border-radius:50%;border:2px solid rgba(255,255,255,.18);
box-shadow:inset 0 0 0 14px rgba(255,255,255,.04),inset 0 0 0 28px rgba(255,255,255,.03);
position:relative}
.art .groove:after{content:"";position:absolute;inset:38%;border-radius:50%;
background:var(--ac);opacity:.7}
.meta{min-width:0;flex:1}
.kicker{margin:0 0 8px;color:var(--ac);font-size:.72rem;letter-spacing:.12em;
text-transform:uppercase;font-weight:650}
h1{margin:0;font-size:clamp(1.5rem,4.5vw,2.15rem);line-height:1.08;
letter-spacing:-.03em;overflow-wrap:anywhere}
.artist{margin:9px 0 0;font-size:1rem;color:var(--tx)}
.album{margin:4px 0 0;font-size:.85rem;color:var(--dim);overflow-wrap:anywhere}
.player{display:flex;align-items:center;gap:15px;margin-top:22px}
.play{flex:none;width:58px;height:58px;border-radius:50%;border:0;background:var(--ac);
color:var(--onac);display:grid;place-items:center;cursor:pointer;
box-shadow:0 8px 24px rgba(210,255,60,.22);transition:transform .15s ease,filter .15s ease}
.play:hover{filter:brightness(1.06)}
.play:active{transform:scale(.96)}
.play:focus-visible{outline:3px solid var(--tx);outline-offset:3px}
.play svg{width:24px;height:24px}
.play .pause{display:none}
.play[data-playing="1"] .pause{display:block}
.play[data-playing="1"] .triangle{display:none}
.trackline{flex:1;min-width:0}
.times{display:flex;justify-content:space-between;gap:10px;
font-size:.78rem;color:var(--dim);font-variant-numeric:tabular-nums;margin-bottom:7px}
.bar{height:4px;border-radius:999px;background:var(--s2);overflow:hidden}
.fill{display:block;height:100%;width:0;background:var(--ac);border-radius:999px;
transition:width .2s linear}
.msg{margin:10px 0 0;font-size:.82rem;color:var(--dim);min-height:1.2em}
.msg.err{color:#ffb4a8}
.actions{display:flex;flex-wrap:wrap;gap:10px;margin-top:22px;padding-top:18px;
border-top:1px solid var(--ln)}
button.ghost{display:inline-flex;align-items:center;gap:8px;border:1px solid var(--ln);
background:transparent;color:var(--tx);border-radius:999px;padding:9px 14px;
font:inherit;font-size:.84rem;font-weight:600;cursor:pointer;min-height:40px}
button.ghost:hover{background:var(--s2)}
button.ghost:focus-visible{outline:2px solid var(--ac);outline-offset:2px}
button.ghost svg{width:16px;height:16px}
a.ghost{display:inline-flex;align-items:center;gap:8px;border:1px solid var(--ln);
background:transparent;color:var(--tx);border-radius:999px;padding:9px 14px;
font-size:.84rem;font-weight:600;text-decoration:none;min-height:40px}
a.ghost:hover{background:var(--s2)}
.note{margin:16px 0 0;font-size:.76rem;color:var(--dim)}
body.embed{padding:10px}
body.embed .card{padding:14px 16px;border-radius:16px}
body.embed .brand{display:none}
body.embed .who{gap:13px}
body.embed .art{width:74px;height:74px;border-radius:10px}
body.embed .kicker,body.embed .album{display:none}
body.embed h1{font-size:1.02rem;line-height:1.15}
body.embed .artist{font-size:.82rem;margin-top:3px}
body.embed .player{margin-top:12px;gap:11px}
body.embed .play{width:44px;height:44px}
body.embed .play svg{width:19px;height:19px}
body.embed .actions,body.embed .note{display:none}
.dead .card{text-align:center;padding:36px 24px}
.deadmsg{max-width:360px;margin:0 auto}
.deadmark{display:block;width:58px;height:58px;margin:0 auto 16px;border-radius:50%;
background:var(--s2);border:1px solid var(--ln);position:relative}
.deadmark:before,.deadmark:after{content:"";position:absolute;left:17px;right:17px;top:28px;
height:2px;background:var(--dim);transform:rotate(45deg)}
.deadmark:after{transform:rotate(-45deg)}
.dead h1{font-size:1.45rem;margin:0 0 8px}
.dead p{margin:0;color:var(--dim);font-size:.92rem;line-height:1.55}
@media(max-width:560px){body{padding:14px 12px;align-items:start}
.card{padding:18px 16px 22px}.who{gap:14px}.art{width:128px;height:128px;border-radius:13px}
h1{font-size:1.4rem}.play{width:54px;height:54px}.brand{margin-bottom:16px}}
@media(prefers-reduced-motion:reduce){.fill{transition:none}.play{transition:none}}
"""

_SCRIPT = """
(function(){
var a=document.getElementById('a'),b=document.getElementById('play'),
f=document.getElementById('fill'),c=document.getElementById('cur'),
d=document.getElementById('dur'),m=document.getElementById('msg'),
src=a.getAttribute('data-src'),total=Number(a.getAttribute('data-duration'))||0,
playing=false,started=false;
function fmt(s){s=Math.max(0,Math.floor(s||0));var x=Math.floor(s/60),y=s%60;
return x+':'+(y<10?'0':'')+y}
function paint(){var t=a.currentTime||0,du=(a.duration&&isFinite(a.duration))?a.duration:total;
c.textContent=fmt(t);if(du){d.textContent=fmt(du);f.style.width=Math.min(100,(t/du)*100)+'%'}}
function fail(){playing=false;b.setAttribute('data-playing','0');b.setAttribute('aria-label','Lire');
m.className='msg err';m.textContent='Lecture impossible pour l\\'instant — réessayez.'}
function toggle(){if(!src){fail();return}
if(!started){started=true;a.src=src;a.load()}
if(playing){a.pause();return}
m.className='msg';m.textContent='';
a.play().catch(fail)}
b.addEventListener('click',toggle);
a.addEventListener('play',function(){playing=true;b.setAttribute('data-playing','1');
b.setAttribute('aria-label','Mettre en pause');paint()});
a.addEventListener('pause',function(){playing=false;b.setAttribute('data-playing','0');
b.setAttribute('aria-label','Lire')});
a.addEventListener('timeupdate',paint);
a.addEventListener('loadedmetadata',paint);
a.addEventListener('ended',function(){playing=false;b.setAttribute('data-playing','0');
b.setAttribute('aria-label','Lire');a.currentTime=0;paint()});
a.addEventListener('error',fail);
var copy=document.getElementById('copy');
if(copy){copy.addEventListener('click',function(){
var done=function(){copy.textContent='Lien copié';setTimeout(function(){
copy.textContent='Copier le lien'},1800)};
if(navigator.clipboard&&navigator.clipboard.writeText){
navigator.clipboard.writeText(location.href).then(done,function(){})}else{done()}})}
paint();
})();
"""


_ART = string.Template(
    "<img src='$src' alt='' width='$w' height='$h' decoding='async' fetchpriority='high'>"
)
_GROOVE = "<span class=groove aria-hidden='true'></span>"
_ARTIST_LINE = string.Template("<p class=artist>$artist</p>")
_ALBUM_LINE = string.Template("<p class=album>$album</p>")
_IMAGE_TAGS = string.Template(
    "<meta property='og:image' content='$src'>"
    "<meta property='og:image:secure_url' content='$src'>"
    "<meta property='og:image:type' content='image/jpeg'>"
    "<meta property='og:image:width' content='$w'>"
    "<meta property='og:image:height' content='$w'>"
    "<meta property='og:image:alt' content='Pochette du titre'>"
)
_TWITTER_IMAGE = string.Template("<meta name='twitter:image' content='$src'>")
_AUDIO_TAGS = string.Template(
    "<meta property='og:audio' content='$src'>"
    "<meta property='og:audio:type' content='audio/ogg'>"
    "<meta property='og:audio:secure_url' content='$src'>"
)
_BACKGROUND = string.Template(
    "<div class=bg style=\"background-image:url('$src')\"></div>"
)
_NO_STREAM = (
    "<div class=deadmsg><h1>Ce titre n’est pas disponible pour l’instant.</h1>"
    "<p>Demandez à la personne qui vous l’a envoyé de recréer le lien.</p></div>"
)

# The card: every value it shows is a ``$name`` filled by ``_render``, and the
# markup itself carries no value. Newlines are kept out of it — a test asserts
# the exact byte budget and the page stays one cheap response.
_BODY = string.Template(
    "<div class=who><div class=art>$art</div><div class=meta>"
    "<p class=kicker>Titre partagé</p><h1>$title</h1>$artist$album"
    "<div class=player>"
    "<button id=play class=play type=button aria-label='Lire'>"
    "<svg class=triangle viewBox='0 0 24 24' aria-hidden='true'>"
    "<path d='M8 5v14l11-7z' fill='currentColor'/></svg>"
    "<svg class=pause viewBox='0 0 24 24' aria-hidden='true'>"
    "<rect x='7' y='5' width='4' height='14' rx='1' fill='currentColor'/>"
    "<rect x='14' y='5' width='4' height='14' rx='1' fill='currentColor'/></svg>"
    "</button>"
    "<div class=trackline><div class=times><span id=cur>0:00</span>"
    "<span id=dur>$duration</span></div>"
    "<div class=bar><span id=fill class=fill></span></div>"
    "<p id=msg class=msg role=status></p></div>"
    "</div></div></div>"
    "<div class=actions>"
    "<button id=copy class=ghost type=button>"
    "<svg viewBox='0 0 24 24' aria-hidden='true' fill='none'"
    " stroke='currentColor' stroke-width='2' stroke-linecap='round'"
    " stroke-linejoin='round'><rect x='9' y='9' width='12' height='12'"
    " rx='2'/><path d='M5 15V5a2 2 0 0 1 2-2h10'/></svg>"
    "Copier le lien</button>"
    "</div>"
    "<p class=note>Écoute libre, sans compte. Le lien est privé et révocable&nbsp;:"
    " celui qui le détient peut écouter ce titre.</p>"
    "<audio id=a preload=none data-src='$stream' data-duration='$seconds'></audio>"
)


def _page_body(track, share):
    token = token_for(share.id)
    stream = _absolute(
        f"{request.script_root.rstrip('/')}/api/shared/track/{token}/stream"
    )
    cover = _absolute(
        f"{request.script_root.rstrip('/')}/api/shared/track/{token}/cover"
        f"?w={PAGE_COVER_WIDTH}"
    )
    title = _track_title(track)
    artist = _artist_name(track)
    album = _album_name(track)
    art = (
        _render(_ART, src=cover, w=PAGE_COVER_WIDTH, h=PAGE_COVER_WIDTH)
        if _cover_available(track)
        else _GROOVE
    )
    return _render(
        _BODY,
        art=art,
        title=title,
        artist=_render(_ARTIST_LINE, artist=artist) if artist else "",
        album=_render(_ALBUM_LINE, album=album) if album else "",
        duration=_mmss(track.duration),
        stream=stream,
        seconds=int(track.duration or 0),
    )


def track_page(token):
    """The public page: ``(response, status)``. A malformed, forged, revoked or
    expired token gets the SAME calm page and status — no oracle."""
    share = _public_share(token)
    if share is None:
        resp = make_response(_render(_DEAD_PAGE, style=_Markup(_STYLE)))
        resp.headers["Content-Type"] = "text/html; charset=utf-8"
        return _public_headers(resp, nonce=None, embed=False), 404

    allowed, retry = _page_limiter.allow(f"{share.id}:{request.remote_addr or ''}")
    if not allowed:
        resp = make_response(_render(_DEAD_PAGE, style=_Markup(_STYLE)))
        resp.status_code = 429
        resp.headers["Retry-After"] = str(retry)
        resp.headers["Content-Type"] = "text/html; charset=utf-8"
        return _public_headers(resp, nonce=None, embed=False), 429

    track = _public_track(share)
    if track is None:
        resp = make_response(_render(_DEAD_PAGE, style=_Markup(_STYLE)))
        resp.headers["Content-Type"] = "text/html; charset=utf-8"
        return _public_headers(resp, nonce=None, embed=False), 404

    embed = request.args.get("embed") == "1"
    nonce = secrets.token_urlsafe(18)
    page_url = _absolute(_share_path(share.id))
    embed_url = page_url + "?embed=1"
    title = _track_title(track)
    artist = _artist_name(track)
    og_title = f"{title} — {artist}" if artist else title
    desc = (
        f"Écoutez « {title} » de {artist} dans le navigateur, sans compte."
        if artist
        else f"Écoutez « {title} » dans le navigateur, sans compte."
    )
    token_url = token_for(share.id)
    cover = _absolute(
        f"{request.script_root.rstrip('/')}/api/shared/track/{token_url}/cover"
        f"?w={OG_COVER_WIDTH}"
    )
    has_cover = _cover_available(track)
    image_tags = ""
    twitter_image = ""
    if has_cover:
        image_tags = _render(_IMAGE_TAGS, src=cover, w=OG_COVER_WIDTH)
        twitter_image = _render(_TWITTER_IMAGE, src=cover)
    stream_url = _absolute(
        f"{request.script_root.rstrip('/')}/api/shared/track/{token_url}/stream"
    )
    streamable = bool(track.path and os.path.isfile(track.path))
    audio_tags = _render(_AUDIO_TAGS, src=stream_url) if streamable else ""
    oembed_url = _absolute("/api/oembed") + "?url=" + quote(page_url, safe="")
    background = (
        _render(_BACKGROUND, src=cover) if has_cover else "<div class=bg></div>"
    )
    body_class = "embed" if embed else ("hasart" if has_cover else "")
    body = _page_body(track, share) if streamable else _NO_STREAM
    page = _render(
        _PAGE,
        page_title=f"{og_title} · Neezer",
        desc=desc,
        page_url=page_url,
        og_title=og_title,
        image_tags=image_tags,
        duration_s=int(track.duration or 0),
        audio_tags=audio_tags,
        twitter_image=twitter_image,
        embed_url=embed_url,
        oembed_url=oembed_url,
        style=_Markup(_STYLE),
        body_class=body_class,
        background=background,
        body=body,
        nonce=nonce,
        script=_Markup(_SCRIPT),
    )
    _touch_used(share)
    resp = make_response(page)
    resp.headers["Content-Type"] = "text/html; charset=utf-8"
    return _public_headers(resp, nonce=nonce, embed=embed), 200


def _public_headers(resp, *, nonce, embed):
    resp.headers["Cache-Control"] = "no-store"
    resp.headers["X-Robots-Tag"] = "noindex, nofollow"
    resp.headers["Referrer-Policy"] = "no-referrer"
    # The same strict policy for every public page: no third-party resource,
    # no inline script without THIS response's nonce, no framing except the
    # compact player an oEmbed consumer embeds.
    script = f"script-src 'nonce-{nonce}'" if nonce else "script-src 'none'"
    frame = "frame-ancestors *" if embed else "frame-ancestors 'none'"
    resp.headers["Content-Security-Policy"] = (
        "default-src 'none'; "
        f"{script}; "
        "style-src 'unsafe-inline'; "
        "img-src 'self' data:; "
        "media-src 'self'; "
        "connect-src 'none'; "
        "object-src 'none'; "
        "base-uri 'none'; "
        "form-action 'none'; "
        f"{frame}"
    )
    # The compact player is the ONE page an oEmbed consumer may frame; the full
    # page keeps the app's SAMEORIGIN. Modern browsers read frame-ancestors, and
    # the legacy header is set to match for the ones that do not.
    resp.headers["X-Frame-Options"] = "ALLOWALL" if embed else "SAMEORIGIN"
    return resp
