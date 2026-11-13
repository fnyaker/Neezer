# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Public read-only playlist links.

The owner of a playlist makes a link (the playlist's "Exporter" sheet); whoever
holds it — no account, no session — sees the playlist and can play the tracks
that are already ARCHIVED on the server, straight from the disk. Nothing is
editable, nothing is downloaded, and nothing asks a provider: a track that is
not archived is listed with ``streamable: false`` and plays only once the owner
archives it. The links are a new unauthenticated surface, so:

- **The token is the capability, and the database cannot hand one out.** It is
  ``<id>.<mac>``, the mac an HMAC of the id under the app's secret key with a
  domain prefix — the same shape as remote.py's. The row names the grant; a copy
  of the table opens nothing, and a secret change kills every link.
- **One live link per playlist is enough.** Creating another returns the one
  already live; revoking kills every live link for that playlist at once.
- **Revocation is immediate**: rows are never cached, and the public responses
  are ``Cache-Control: no-store`` + ``X-Robots-Tag: noindex``.
- **Streams are rate-limited** per token and client IP: 120 requests / 10 min.
- **A private upload of somebody else is never listed and never served**, and
  no public route ever reaches the provider (an unarchived Deezer track has no
  public cover either — the page shows the groove placeholder).
"""

from __future__ import annotations

import base64
import hashlib
import hmac
import logging
import os
import re
import secrets
import threading
import time
import uuid
from collections import deque
from datetime import timedelta

from flask import current_app, jsonify, request, send_file

from ..db import Album, Artist, Playlist, PlaylistShare, PlaylistTrack, Track, User, now
from . import (
    _credits_by_raw_id,
    _display_artist,
    _quality_fields,
    _resolve_db_playlist,
    _row_columns,
    _uuid_text,
    _valid_id,
    _youtube_fields,
    cover_response,
    login_required,
    webapi,
)
from .refs import cover_url_for

logger = logging.getLogger(__name__)

TTL_MIN = 3600  # s: the shortest link one may ask for
TTL_MAX = 30 * 24 * 3600  # s: a month at most
TOUCH_EVERY = 60.0  # s between last_used writes for one link

_DOMAIN = b"ns-playlist-share:"
_TOKEN_RE = re.compile(r"\A([A-Za-z0-9_-]{16,24})\.([A-Za-z0-9_-]{24})\Z")


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
    """The share id a token names, or None when it is not a token this process
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
    row = PlaylistShare.get_or_none(PlaylistShare.id == share_id)
    return row if _alive(row) else None


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
        PlaylistShare.update(last_used=now()).where(PlaylistShare.id == row.id).execute()
    except Exception:  # a timestamp is never worth failing the request over
        logger.debug("Could not stamp last_used on %s", row.id, exc_info=True)


def _no_store(resp):
    """A public answer must be un-cacheable everywhere (revocation is immediate)
    and invisible to crawlers."""
    resp.headers["Cache-Control"] = "no-store"
    resp.headers["X-Robots-Tag"] = "noindex"
    return resp


def _share_url(share_id: str) -> str:
    """The short link path, under whatever prefix the app is mounted at."""
    return f"{request.script_root.rstrip('/')}/pl/{token_for(share_id)}"


def _link_payload(row) -> dict:
    return {
        "token": token_for(row.id),
        "url": _share_url(row.id),
        "created": row.created.isoformat() if row.created else None,
        "expires": row.expires.isoformat() if row.expires else None,
    }


# -- the stream limiter ------------------------------------------------------------


class _RequestLimiter:
    """A sliding window of requests per key, for the public stream.

    In-process, like auth_limiter: the server runs as one gunicorn worker (see
    the Dockerfile CMD), so one process holds the whole view.
    """

    def __init__(self, max_requests=120, window=600):
        self.max_requests = max_requests
        self.window = window
        self._hits: dict = {}
        self._lock = threading.Lock()

    def allow(self, key):
        """``(allowed, retry_after_s)``: False means the key is over the cap,
        with the seconds until its oldest hit leaves the window."""
        t = time.monotonic()
        with self._lock:
            dq = self._hits.get(key)
            created = dq is None
            if created:
                dq = self._hits[key] = deque()
            while dq and t - dq[0] > self.window:
                dq.popleft()
            if not dq and not created:
                # The window emptied it: start the key afresh rather than
                # counting an orphaned deque nobody can see.
                self._hits.pop(key, None)
                dq = self._hits[key] = deque()
            if len(dq) >= self.max_requests:
                return False, max(1, int(self.window - (t - dq[0])) + 1)
            dq.append(t)
            if len(self._hits) > 4096:  # a bounded table: forget dead keys
                self._hits = {k: v for k, v in self._hits.items() if v}
            return True, 0

    def clear(self):
        with self._lock:
            self._hits.clear()


_stream_limiter = _RequestLimiter(max_requests=120, window=600)

# -- owner side ---------------------------------------------------------------------


def _owned_playlist(pid):
    """The playlist, when the caller is its owner or an admin. Somebody else's
    playlist reads exactly like one that does not exist."""
    pl = _resolve_db_playlist(pid)
    if pl is None:
        return None
    user = request.webuser
    if not (user.admin or pl.user_id == user.id):
        return None
    return pl


@webapi.route("/playlist/<playlist_id>/share")
@login_required
def playlist_share(playlist_id):
    """The playlist's live public link, or ``link: null``."""
    pl = _owned_playlist(playlist_id)
    if pl is None:
        return jsonify({"error": "not found"}), 404
    row = (
        PlaylistShare.select()
        .where(PlaylistShare.playlist == pl)
        .order_by(PlaylistShare.created.desc())
        .first()
    )
    if not _alive(row):
        return jsonify({"link": None})
    return jsonify({"link": _link_payload(row)})


@webapi.route("/playlist/<playlist_id>/share", methods=["POST"])
@login_required
def playlist_share_create(playlist_id):
    """Create the public link (``ttl`` seconds, 3600..30 days, null = forever),
    or hand back the live link that already exists — one per playlist is
    enough."""
    pl = _owned_playlist(playlist_id)
    if pl is None:
        return jsonify({"error": "not found"}), 404
    ttl = None
    data = request.get_json(silent=True) or {}
    if data.get("ttl") is not None:
        try:
            ttl = int(data["ttl"])
        except (TypeError, ValueError):
            return jsonify({"error": "ttl must be a number of seconds"}), 400
        if not TTL_MIN <= ttl <= TTL_MAX:
            return (
                jsonify({"error": f"ttl must be between {TTL_MIN} and {TTL_MAX} seconds"}),
                400,
            )
    for row in (
        PlaylistShare.select()
        .where(PlaylistShare.playlist == pl)
        .order_by(PlaylistShare.created.desc())
    ):
        if _alive(row):
            return jsonify({"link": _link_payload(row)})
    expires = now() + timedelta(seconds=ttl) if ttl else None
    row = PlaylistShare.create(
        id=secrets.token_urlsafe(12),
        playlist=pl,
        user=request.webuser,
        expires=expires,
    )
    return jsonify({"link": _link_payload(row)})


@webapi.route("/playlist/<playlist_id>/share", methods=["DELETE"])
@login_required
def playlist_share_revoke(playlist_id):
    """Revoke every live link for this playlist. Dead on the next request."""
    pl = _owned_playlist(playlist_id)
    if pl is None:
        return jsonify({"error": "not found"}), 404
    n = (
        PlaylistShare.update(revoked=now())
        .where(PlaylistShare.playlist == pl, PlaylistShare.revoked.is_null(True))
        .execute()
    )
    return jsonify({"revoked": n})


# -- the public page's data ----------------------------------------------------------


def _visible_members(share):
    """The playlist's track ids that a link holder may see: shared-library rows
    plus the share owner's own uploads — never somebody else's private upload."""
    return (
        PlaylistTrack.select(PlaylistTrack.track)
        .join(Track, on=(Track.id == PlaylistTrack.track))
        .where(
            PlaylistTrack.playlist == share.playlist_id,
            Track.owner.is_null(True) | (Track.owner == share.user_id),
        )
    )


def _member_albums(share):
    return (
        Album.select(Album.id)
        .join(Track, on=(Track.album == Album.id))
        .where(Track.id.in_(_visible_members(share)))
    )


def _member_track(share, mid):
    """The playlist's Track for a public media id (a Deezer id, or the row UUID
    of a local track), respecting the private-upload exclusion. None when the id
    is not a member."""
    members = _visible_members(share)
    if _valid_id(mid):
        return Track.get_or_none((Track.deezer_id == mid) & Track.id.in_(members))
    try:
        key = uuid.UUID(str(mid))
    except (ValueError, AttributeError):
        return None
    return Track.get_or_none((Track.id == key) & Track.id.in_(members))


def _member_album(share, mid):
    """The playlist's Album for a UUID, or None."""
    try:
        key = uuid.UUID(str(mid))
    except (ValueError, AttributeError):
        return None
    return Album.get_or_none((Album.id == key) & Album.id.in_(_member_albums(share)))


def _public_payload(share, token):
    """``{playlist, tracks}`` for the shared page, straight from the database.

    Reads the rows as tuples (the big-list path — a 4000-track playlist costs
    one SELECT, not thousands of model hydrations), excludes other people's
    private uploads entirely, and marks each track with its public media id,
    its capability cover URL and whether its file is on disk.
    """
    pl = Playlist.get_or_none(Playlist.id == share.playlist_id)
    if pl is None:  # the playlist itself is gone
        return None
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
    rows = list(query.select(*_row_columns(), Track.owner).tuples())
    within = query.select(Track.id).order_by().where(Track.deezer_id.is_null(False))
    credits = _credits_by_raw_id([r[0] for r in rows if r[1] is not None], within)
    tracks = []
    for r in rows:
        (
            tid, dz, title, duration, gone, gain, ytid, bitrate, released,
            aid, adz, aname, alid, aldz, alname, md5, path, modified, owner_id,
        ) = r
        if owner_id is not None and _uuid_text(owner_id) != str(share.user_id):
            continue  # somebody else's private upload: never listed
        if dz is None:
            uid = _uuid_text(tid)
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
                "artist": {"deezer_id": _uuid_text(aid), "name": aname},
                "artists": [{"deezer_id": _uuid_text(aid), "name": aname, "role": "Main"}],
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
        else:
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
        mid = str(dz) if dz else _uuid_text(tid)
        row["mid"] = mid
        row["cover"] = f"/api/shared/playlist/{token}/cover/{mid}"
        # "Archived" means the bytes are on disk NOW, not merely that the row
        # has been through an archive once: a moved or deleted file must show
        # as unplayable instead of handing the page a URL that will 409.
        row["streamable"] = bool(modified and path and os.path.isfile(path))
        tracks.append(row)
    owner = User.get_or_none(User.id == pl.user_id)
    return {
        "playlist": {
            "title": pl.name,
            "description": pl.comment or "",
            "cover": tracks[0]["cover"] if tracks else None,
            "nb_tracks": len(tracks),
            "owner": owner.name if owner else "",
        },
        "tracks": tracks,
    }


# -- public routes -------------------------------------------------------------------


@webapi.route("/shared/playlist/<token>")
def shared_playlist(token):
    """The public page's data. No login, no session, no provider."""
    share_id = _parse_token(token)
    share = _live_row(share_id) if share_id else None
    if share is None:
        return _no_store(jsonify({"error": "not found"})), 404
    _touch_used(share)
    payload = _public_payload(share, token_for(share_id))
    if payload is None:
        return _no_store(jsonify({"error": "not found"})), 404
    return _no_store(jsonify(payload))


@webapi.route("/shared/playlist/<token>/cover/<mid>")
def shared_playlist_cover(token, mid):
    """A cover, for a track or album that belongs to this playlist and whose
    art the server already holds. Never a provider call on a public page: an
    unarchived Deezer track simply has no public cover yet."""
    share_id = _parse_token(token)
    share = _live_row(share_id) if share_id else None
    if share is None:
        return _no_store(jsonify({"error": "not found"})), 404
    track = _member_track(share, mid)
    if track is None:
        album = _member_album(share, mid)
        if album is None:
            return _no_store(jsonify({"error": "not found"})), 404
        return cover_response(mid, vetted=True)
    if track.deezer_id is not None and (track.last_modification or 0) <= 0:
        return _no_store(jsonify({"error": "not found"})), 404
    return cover_response(mid, vetted=True)


@webapi.route("/shared/playlist/<token>/stream/<mid>")
def shared_playlist_stream(token, mid):
    """The ARCHIVED file, and nothing else: never an archive-on-play, never a
    transcode, never a provider call. Range requests work (``send_file``)."""
    share_id = _parse_token(token)
    share = _live_row(share_id) if share_id else None
    if share is None:
        return _no_store(jsonify({"error": "not found"})), 404
    allowed, retry = _stream_limiter.allow(f"{share.id}:{request.remote_addr or ''}")
    if not allowed:
        resp = _no_store(jsonify({"error": "too many requests"}))
        resp.status_code = 429
        resp.headers["Retry-After"] = str(retry)
        return resp
    track = _member_track(share, mid)
    if track is None:
        return _no_store(jsonify({"error": "not found"})), 404
    if not (
        track.path
        and os.path.isfile(track.path)
        and (track.last_modification or 0) > 0
    ):
        resp = _no_store(jsonify({"error": "not archived"}))
        resp.status_code = 409
        return resp
    _touch_used(share)
    return send_file(track.path, mimetype=track.mimetype, conditional=True)


def preview(token):
    """``(title, description)`` for the short link's HTML page, or None when the
    link is dead — the page must never raise."""
    share_id = _parse_token(token)
    share = _live_row(share_id) if share_id else None
    if share is None:
        return None
    pl = Playlist.get_or_none(Playlist.id == share.playlist_id)
    if pl is None:
        return None
    owner = User.get_or_none(User.id == share.user_id)
    owner_name = owner.name if owner else ""
    title = f"Playlist « {pl.name} »" + (f" de {owner_name}" if owner_name else "")
    count = _visible_members(share).count()
    desc = (pl.comment or "").strip() or (
        f"{count} titre{'s' if count != 1 else ''} en écoute libre, rien à installer."
    )
    return title, desc
