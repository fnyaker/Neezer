# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""The server's LAN routes: enrolment, roster, devices, friends, settings.

The logic is ``neezer/lan.py`` (the LAN key, the credential, the roster, the
revocation list); this module is the HTTP shape of it — ``docs/agents/lan-nodes.md``
*Server*, the route table — and nothing else.

**Every route is ``@login_required`` and ``NEVER`` for a remote-control
session.** A lent link plays the owner's music; it must never enrol a device,
revoke one, befriend somebody or read the trust chain: those are the owner's own
hand, exactly like making a remote link. ``LAN_POLICY`` is merged into
``remote.POLICY`` when the blueprint registers (the way ``invites.py`` does), so
``test_remote``'s "every endpoint is classified" keeps holding for these routes.

Bodies are capped at 8 KiB and every field is validated before it reaches the
database; a refusal is one French sentence and a machine code, never a
distinction an unauthenticated caller could use to learn who exists. Nothing
here logs a key, a credential, a roster or a cookie.
"""

from __future__ import annotations

import json
import logging

from flask import Blueprint, jsonify, request, session

from .. import audit, lan
from ..ratelimit import WindowLimiter
from . import _is_admin, login_required, same_origin_guard
from .remote import NEVER, POLICY

logger = logging.getLogger(__name__)

MAX_BODY = 8 * 1024

#: Nothing here is ever lent to a remote-control session.
LAN_POLICY = {
    "lan_status": NEVER,
    "lan_enroll": NEVER,
    "lan_roster": NEVER,
    "lan_devices": NEVER,
    "lan_device_rename": NEVER,
    "lan_device_revoke": NEVER,
    "lan_friends": NEVER,
    "lan_add_friend": NEVER,
    "lan_set_friend": NEVER,
    "lan_remove_friend": NEVER,
    "lan_settings": NEVER,
    "lan_rotate": NEVER,
}


class _LanBlueprint(Blueprint):
    """Registers its policy with the remote-control table as it is added."""

    def register(self, app, options):
        super().register(app, options)
        POLICY.update(LAN_POLICY)


lan_api = _LanBlueprint("lan_api", __name__)

# The note's two budgets: 30 enrolments and 30 befriending attempts per person
# per hour. Both are SUCCESS-side counters (a refused call records nothing).
enroll_limiter = WindowLimiter(max_events=30, window=3600)
friend_limiter = WindowLimiter(max_events=30, window=3600)


def _reset_for_tests():
    enroll_limiter.clear()
    friend_limiter.clear()
    lan._reset_for_tests()


@lan_api.before_request
def reject_cross_site():
    return same_origin_guard()


@lan_api.before_request
def _cap_body():
    if request.method in ("POST", "PUT", "PATCH"):
        try:
            request.max_content_length = MAX_BODY
        except AttributeError:
            pass


@lan_api.errorhandler(Exception)
def _json_error(exc):
    """A JSON answer for every failure, and never the exception's text."""
    from werkzeug.exceptions import HTTPException

    if isinstance(exc, HTTPException):
        if exc.code == 413:
            return _refused(
                lan.Refused("too_large", "Requête trop volumineuse.", 413)
            )
        return _refused(
            lan.Refused(
                "http_%s" % exc.code,
                "Requête invalide.",
                exc.code,
            )
        )
    logger.error("Unhandled LAN error on %s %s", request.method, request.path,
                 exc_info=True)
    return _refused(lan.Refused("server_error", "Erreur serveur.", 500))


def _refused(exc):
    payload = {"error": exc.message, "code": exc.code}
    if exc.field is not None:
        payload["field"] = exc.field
    resp = jsonify(payload)
    resp.status_code = exc.status
    if exc.retry_after is not None:
        resp.headers["Retry-After"] = str(max(1, int(exc.retry_after)))
    resp.headers["Cache-Control"] = "no-store"
    return resp


def _rate_limited(retry_after):
    return _refused(
        lan.Refused(
            "rate_limited",
            "Trop de tentatives. Réessaie dans quelques minutes.",
            429,
            retry_after=retry_after,
        )
    )


def _forbidden():
    return _refused(lan.Refused("forbidden", "Accès refusé.", 403))


def _admin_guard():
    return None if _is_admin() else _forbidden()


def _json_body():
    """The request's JSON object, capped BEFORE it is parsed."""
    length = request.content_length
    if length is not None and length > MAX_BODY:
        return None, _refused(
            lan.Refused("too_large", "Requête trop volumineuse.", 413)
        )
    if not request.is_json:
        return None, _refused(
            lan.Refused("invalid_body", "Requête invalide.", 400)
        )
    if length is None:
        try:
            raw = request.stream.read(MAX_BODY + 1)
        except Exception:
            return None, _refused(
                lan.Refused("invalid_body", "Requête invalide.", 400)
            )
        if len(raw) > MAX_BODY:
            return None, _refused(
                lan.Refused("too_large", "Requête trop volumineuse.", 413)
            )
        if not raw.strip():
            return None, _refused(
                lan.Refused("invalid_body", "Requête invalide.", 400)
            )
        try:
            data = json.loads(raw)
        except (ValueError, UnicodeDecodeError):
            return None, _refused(
                lan.Refused("invalid_body", "Requête invalide.", 400)
            )
    else:
        data = request.get_json(silent=True)
    if not isinstance(data, dict):
        return None, _refused(lan.Refused("invalid_body", "Requête invalide.", 400))
    return data, None


def _current_device_id():
    """The device this browser enrolled, from the signed session cookie."""
    value = session.get("lan_device")
    return value if isinstance(value, str) else None


def _no_store(resp):
    resp.headers["Cache-Control"] = "no-store"
    return resp


def _etag_matches(header, etag):
    if not header:
        return False
    for candidate in header.split(","):
        candidate = candidate.strip()
        if candidate.startswith("W/"):
            candidate = candidate[2:]
        if candidate in (etag, "*"):
            return True
    return False


# -- the feature's state ----------------------------------------------------------


@lan_api.route("/lan/status")
@login_required
def lan_status():
    """The feature probe, and what an enrolment needs before it starts.

    ``enabled=false`` is this server's own switch: every client then hides the
    feature, exactly as it does for a server too old to have the route.
    """
    return jsonify(lan.status_json(request.webuser))


# -- enrolment ---------------------------------------------------------------------


@lan_api.route("/lan/enroll", methods=["POST"])
@login_required
def lan_enroll():
    user = request.webuser
    data, error = _json_body()
    if error is not None:
        return error
    if not lan.enabled():
        return _refused(
            lan.Refused(
                "disabled",
                "L'échange entre appareils proches est désactivé sur ce serveur.",
                403,
            )
        )
    allowed, retry_after = enroll_limiter.allow(f"enroll:{user.id}")
    if not allowed:
        return _rate_limited(retry_after)
    try:
        answer = lan.enroll(
            user,
            pubkey=data.get("pubkey"),
            name=data.get("name"),
            platform=data.get("platform"),
            at=data.get("at"),
            proof=data.get("proof"),
            device_id=data.get("device_id"),
        )
    except lan.Refused as exc:
        return _refused(exc)
    # The device id is not a secret (it travels inside the credential); keeping
    # it in the signed session cookie is what makes DELETE .../self and
    # ``current`` on the device list able to name THIS device.
    session["lan_device"] = answer["device_id"]
    logger.info("LAN device enrolled for %s", user.name)
    return _no_store(jsonify(answer))


# -- roster ------------------------------------------------------------------------


@lan_api.route("/lan/roster")
@login_required
def lan_roster():
    """The signed roster and revocation list, with the note's ETag.

    The cheap half runs first (the serials and the revoked keys, no JSON and no
    signature): a device that already holds this roster gets an empty 304, which
    is the point — it asks on every launch.
    """
    user = request.webuser
    state = lan.roster_state(user)
    etag = lan.roster_etag(state)
    if _etag_matches(request.headers.get("If-None-Match"), etag):
        resp = jsonify(None)
        resp.status_code = 304
        resp.set_data(b"")
        resp.headers["ETag"] = etag
        resp.headers["Cache-Control"] = "private, no-cache"
        return resp
    body = lan.roster_answer(user, state)
    resp = jsonify(body)
    resp.headers["ETag"] = etag
    resp.headers["Cache-Control"] = "private, no-cache"
    return resp


# -- devices -----------------------------------------------------------------------


@lan_api.route("/lan/devices")
@login_required
def lan_devices():
    return jsonify(lan.devices_json(request.webuser, _current_device_id()))


@lan_api.route("/lan/devices/<device_id>", methods=["PATCH"])
@login_required
def lan_device_rename(device_id):
    data, error = _json_body()
    if error is not None:
        return error
    try:
        device = lan.rename_device(
            request.webuser, device_id, data.get("name"), _current_device_id()
        )
    except lan.Refused as exc:
        return _refused(exc)
    return jsonify(device)


@lan_api.route("/lan/devices/<device_id>", methods=["DELETE"])
@login_required
def lan_device_revoke(device_id):
    user = request.webuser
    try:
        device = lan.revoke_device(user, device_id, _current_device_id())
    except lan.Refused as exc:
        return _refused(exc)
    audit.record("lan.device_revoke", user, device.id, device.name)
    logger.info("LAN device revoked for %s", user.name)
    resp = jsonify(None)
    resp.status_code = 204
    resp.set_data(b"")
    return resp


# -- trusted friends ---------------------------------------------------------------


@lan_api.route("/lan/friends")
@login_required
def lan_friends():
    return jsonify(lan.friends_json(request.webuser))


@lan_api.route("/lan/friends", methods=["POST"])
@login_required
def lan_add_friend():
    data, error = _json_body()
    if error is not None:
        return error
    allowed, retry_after = friend_limiter.allow(f"friend:{request.webuser.id}")
    if not allowed:
        return _rate_limited(retry_after)
    try:
        friend = lan.add_friend(request.webuser, data.get("name"))
    except lan.Refused as exc:
        return _refused(exc)
    return jsonify(friend)


@lan_api.route("/lan/friends/<friend_id>", methods=["PATCH"])
@login_required
def lan_set_friend(friend_id):
    data, error = _json_body()
    if error is not None:
        return error
    all_ = data.get("all")
    if all_ is not None and not isinstance(all_, bool):
        return _refused(
            lan.Refused(
                "invalid_all",
                "Le partage total doit être vrai ou faux.",
                400,
                field="all",
            )
        )
    try:
        friend = lan.set_scope(
            request.webuser, friend_id, all_=all_, playlists=data.get("playlists")
        )
    except lan.Refused as exc:
        return _refused(exc)
    return jsonify(friend)


@lan_api.route("/lan/friends/<friend_id>", methods=["DELETE"])
@login_required
def lan_remove_friend(friend_id):
    try:
        lan.remove_friend(request.webuser, friend_id)
    except lan.Refused as exc:
        return _refused(exc)
    resp = jsonify(None)
    resp.status_code = 204
    resp.set_data(b"")
    return resp


# -- administration (rule 6) -------------------------------------------------------


@lan_api.route("/lan/settings", methods=["GET", "PUT"])
@login_required
def lan_settings():
    forbidden = _admin_guard()
    if forbidden is not None:
        return forbidden
    if request.method == "GET":
        return jsonify(lan.settings_json())
    data, error = _json_body()
    if error is not None:
        return error
    try:
        return jsonify(lan.save_settings(data))
    except lan.Refused as exc:
        return _refused(exc)


@lan_api.route("/lan/rotate", methods=["POST"])
@login_required
def lan_rotate():
    forbidden = _admin_guard()
    if forbidden is not None:
        return forbidden
    key = lan.rotate()
    audit.record("lan.rotate", request.webuser, key.kid.hex())
    logger.info("admin %s rotated the LAN key", request.webuser.name)
    return _no_store(jsonify({"kid": key.kid.hex()}))
