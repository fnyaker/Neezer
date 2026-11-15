# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Invitation administration and the two public redemption routes.

Administrators mint single-use links; a link is shown once, then only its
SHA-256 is kept (``neezer/invites.py``). The public routes take the token in
the JSON body, never in the path, so it cannot reach an access log. Every
invalid token, expired, revoked or already used gets the same 404 body, and the
redeem is one conditional UPDATE inside the transaction that creates the
account: two submissions have one winner and one account.

The redeem opens the session like ``POST /api/login`` (a spent link is a
credential, not a cookie), and the account is never an admin.
"""

from __future__ import annotations

import hashlib
import hmac
import json
import logging
import threading
import time
from collections import OrderedDict
from datetime import datetime

from flask import Blueprint, current_app, jsonify, request, session

from .. import invites
from ..db import Invite, User, db, now
from ..managers.user import (
    InvalidUserName,
    PasswordMismatch,
    UserManager,
    UserNameTaken,
    WeakPassword,
    check_invite_password,
    validate_name,
)
from ..ratelimit import RateLimiter, WindowLimiter
from . import _is_admin, login_required, same_origin_guard
from .remote import INVITE_POLICY, POLICY

logger = logging.getLogger(__name__)


class _InvitesBlueprint(Blueprint):
    def register(self, app, options):
        super().register(app, options)
        POLICY.update(INVITE_POLICY)


invites_api = _InvitesBlueprint("invites_api", __name__)

MAX_BODY = 8 * 1024
PAGE_DEFAULT = 50
PAGE_MAX = 200
STATES = ("pending", "used", "expired", "revoked")
REPLAY_TTL = 600.0
REPLAY_MAX = 1024

inspect_limiter = RateLimiter(max_attempts=60, window=300)
redeem_limiter = RateLimiter(max_attempts=5, window=900)
create_limiter = WindowLimiter(max_events=20, window=3600)

_replays = OrderedDict()
_replays_lock = threading.Lock()


class _Gone(Exception):
    pass


class _Used(Exception):
    pass


class _LiveCap(Exception):
    pass


def _reset_for_tests():
    global inspect_limiter, redeem_limiter
    inspect_limiter = RateLimiter(max_attempts=60, window=300)
    redeem_limiter = RateLimiter(max_attempts=5, window=900)
    create_limiter.clear()
    UserManager.reset_last_login_cache()
    with _replays_lock:
        _replays.clear()


@invites_api.before_request
def reject_cross_site():
    return same_origin_guard()


@invites_api.before_request
def _cap_body():
    if request.method in ("POST", "PUT", "PATCH"):
        try:
            request.max_content_length = MAX_BODY
        except AttributeError:
            pass


@invites_api.errorhandler(Exception)
def _json_error(exc):
    from werkzeug.exceptions import HTTPException

    if isinstance(exc, HTTPException):
        if exc.code == 413:
            return _failure("Requête trop volumineuse.", "too_large", 413)
        return _failure(exc.name, "http_%s" % exc.code, exc.code)
    logger.error("Unhandled error on %s %s", request.method, request.path, exc_info=True)
    return _failure("Erreur serveur.", "server_error", 500)


def _no_store(resp):
    resp.headers["Cache-Control"] = "no-store"
    resp.headers["Pragma"] = "no-cache"
    resp.headers["X-Robots-Tag"] = "noindex"
    resp.headers["Referrer-Policy"] = "no-referrer"
    return resp


def _failure(message, code, status, field=None, retry_after=None, reason=None):
    payload = {"error": message, "code": code}
    if field is not None:
        payload["field"] = field
    if reason is not None:
        payload["reason"] = reason
    resp = jsonify(payload)
    resp.status_code = status
    if retry_after is not None:
        resp.headers["Retry-After"] = str(max(1, int(retry_after)))
    return resp


def _rate_limited(retry_after):
    return _failure(
        "Trop de tentatives. Réessaie dans quelques minutes.",
        "rate_limited",
        429,
        retry_after=retry_after,
    )


def _invite_invalid():
    return _no_store(
        _failure("Cette invitation n'est plus valide.", "invite_invalid", 404)
    )


def _invite_not_found():
    return _failure("Cette invitation n'existe pas.", "invite_not_found", 404)


def _invite_used():
    return _failure("Cette invitation a déjà servi.", "invite_used", 409)


def _live_cap():
    return _failure(
        "Trop d'invitations en attente. Révoque-en ou attends leur expiration.",
        "rate_limited",
        429,
        retry_after=3600,
        reason="invite_cap",
    )


def _forbidden():
    return _failure("Accès refusé.", "forbidden", 403)


def _admin_guard():
    return None if _is_admin() else _forbidden()


def _json_body():
    length = request.content_length
    if length is not None and length > MAX_BODY:
        return None, _failure("Requête trop volumineuse.", "too_large", 413)
    if not request.is_json:
        return None, _failure("Requête invalide.", "invalid_body", 400)
    if length is None:
        try:
            raw = request.stream.read(MAX_BODY + 1)
        except Exception:
            return None, _failure("Requête invalide.", "invalid_body", 400)
        if len(raw) > MAX_BODY:
            return None, _failure("Requête trop volumineuse.", "too_large", 413)
        if not raw.strip():
            return None, _failure("Requête invalide.", "invalid_body", 400)
        try:
            data = json.loads(raw)
        except (ValueError, UnicodeDecodeError):
            return None, _failure("Requête invalide.", "invalid_body", 400)
    else:
        data = request.get_json(silent=True)
    if not isinstance(data, dict):
        return None, _failure("Requête invalide.", "invalid_body", 400)
    return data, None


def _iso(value):
    return value.isoformat(timespec="seconds") if value is not None else None


def _note(data):
    raw = data.get("note")
    if raw is None:
        return None, None
    if not isinstance(raw, str):
        return None, _failure(
            "La note doit faire 64 caractères au plus.", "invalid_note", 400, field="note"
        )
    note = raw.strip()
    if not note:
        return None, None
    if len(note) > invites.NOTE_MAX:
        return None, _failure(
            "La note doit faire 64 caractères au plus.", "invalid_note", 400, field="note"
        )
    return note, None


def _expires_in(data):
    raw = data.get("expires_in")
    if raw is None:
        return invites.TTL_DEFAULT, None
    if isinstance(raw, bool) or not isinstance(raw, int):
        return None, _failure(
            "La durée doit être exprimée en secondes.",
            "invalid_expires_in",
            400,
            field="expires_in",
        )
    if raw < invites.TTL_MIN or raw > invites.TTL_MAX:
        return None, _failure(
            "La durée doit être comprise entre 1 heure et 30 jours.",
            "invalid_expires_in",
            400,
            field="expires_in",
        )
    return raw, None


def _invite_json(row, at):
    return {
        "id": row.id,
        "note": row.note,
        "created": _iso(row.created),
        "expires": _iso(row.expires),
        "state": invites.state(row, at),
    }


def _names_for(rows):
    ids = {row.created_by_id for row in rows} | {row.used_by_id for row in rows}
    ids.discard(None)
    if not ids:
        return {}
    return {
        user.id: user.name
        for user in User.select(User.id, User.name).where(User.id.in_(ids))
    }


def _row_json(row, names, at):
    out = _invite_json(row, at)
    out["created_by"] = names.get(row.created_by_id)
    out["used_by"] = names.get(row.used_by_id)
    out["used_at"] = _iso(row.used_at)
    return out


def _row_or_none(invite_id):
    if not invites.valid_id(invite_id):
        return None
    return Invite.get_or_none(Invite.id == invite_id)


def _token_key(token):
    if not isinstance(token, str) or not token.strip():
        return "invite:empty"
    return "invite:" + hashlib.sha256(
        token.strip().encode("utf-8", "replace")
    ).hexdigest()[:32]


def _client_ip():
    return request.remote_addr or "?"


def _replay_key(invite_id, request_id):
    digest = hashlib.sha256(request_id.encode("utf-8", "replace")).hexdigest()[:32]
    return invite_id + ":" + digest


def _remember_redeem(invite_id, request_id, user_id):
    if not isinstance(request_id, str) or not request_id or len(request_id) > 128:
        return
    key = _replay_key(invite_id, request_id)
    with _replays_lock:
        _replays[key] = (user_id, time.monotonic() + REPLAY_TTL)
        _replays.move_to_end(key)
        while len(_replays) > REPLAY_MAX:
            _replays.popitem(last=False)


def _replay_user(row, request_id):
    if row is None or row.used_at is None or row.used_by_id is None:
        return None
    if not isinstance(request_id, str) or not request_id or len(request_id) > 128:
        return None
    key = _replay_key(row.id, request_id)
    with _replays_lock:
        entry = _replays.get(key)
        if entry is None:
            return None
        user_id, expires = entry
        if expires < time.monotonic():
            _replays.pop(key, None)
            return None
        _replays.move_to_end(key)
    if user_id != row.used_by_id:
        return None
    return User.get_or_none(User.id == user_id)


def _validate_form(data):
    username = data.get("username")
    if not isinstance(username, str):
        return None, _failure(
            "Le nom doit faire 3 à 32 caractères : a–z, 0–9, point, tiret, tiret bas.",
            "username_invalid",
            400,
            field="username",
        )
    try:
        key = validate_name(username)
    except (InvalidUserName, TypeError):
        return None, _failure(
            "Le nom doit faire 3 à 32 caractères : a–z, 0–9, point, tiret, tiret bas.",
            "username_invalid",
            400,
            field="username",
        )
    try:
        validate_name(username, reserved=True)
    except InvalidUserName:
        return None, _failure(
            "Ce nom d'utilisateur n'est pas disponible.",
            "username_unavailable",
            400,
            field="username",
        )
    password = data.get("password")
    try:
        check_invite_password(key, password)
    except WeakPassword:
        return None, _failure(
            "Ce mot de passe est trop faible : 12 caractères minimum, pas un mot de passe courant.",
            "password_weak",
            400,
            field="password",
        )
    confirm = data.get("password_confirm")
    if not isinstance(confirm, str) or not hmac.compare_digest(
        password.encode("utf-8"), confirm.encode("utf-8")
    ):
        return None, _failure(
            "Les deux mots de passe ne correspondent pas.",
            "password_mismatch",
            400,
            field="password_confirm",
        )
    return key, None


def _session_response(user, status):
    from .accessapi import me_access
    from .sourcesapi import sources_brief

    session.clear()
    session["uid"] = str(user.id)
    session["epoch"] = user.session_epoch or 0
    session.permanent = True
    resp = jsonify(
        {
            "user": {
                "name": user.name,
                "admin": user.admin,
                "onboarding": user.onboarding_step or "done",
                "access": me_access(user),
                "sources": sources_brief(),
            }
        }
    )
    resp.status_code = status
    return _no_store(resp)


@invites_api.route("/invites/inspect", methods=["POST"])
def invite_inspect():
    data, error = _json_body()
    if error is not None:
        return error
    token = data.get("token")
    token = token if isinstance(token, str) else ""
    ip = _client_ip()
    if inspect_limiter.is_blocked(ip):
        return _rate_limited(inspect_limiter.window)
    row = invites.find(token)
    if row is None:
        inspect_limiter.record_failure(ip)
        return _invite_invalid()
    return _no_store(
        jsonify(
            {
                "valid": True,
                "expires": _iso(row.expires),
                "label": "Invitation",
            }
        )
    )


@invites_api.route("/invites/redeem", methods=["POST"])
def invite_redeem():
    data, error = _json_body()
    if error is not None:
        return error
    token = data.get("token")
    token = token if isinstance(token, str) else ""
    ip = _client_ip()
    token_key = _token_key(token)
    if redeem_limiter.is_blocked(redeem_limiter.user_key(token_key)):
        return _rate_limited(redeem_limiter.window)

    row = invites.find_any(token)
    request_id = data.get("request_id")
    if row is None:
        redeem_limiter.record_failure(ip, token_key)
        return _invite_invalid()
    if not invites.alive(row):
        replay = _replay_user(row, request_id)
        if replay is not None:
            return _session_response(replay, 201)
        redeem_limiter.record_failure(ip, token_key)
        return _invite_invalid()

    key, error = _validate_form(data)
    if error is not None:
        redeem_limiter.record_failure(ip, token_key)
        return error

    at = now()
    try:
        with db.atomic():
            if invites.claim(row.id, at) != 1:
                raise _Gone
            user = UserManager.create_from_invite(
                key, data.get("password"), data.get("password_confirm"), at=at
            )
            invites.mark_used_by(row.id, user)
    except _Gone:
        current = Invite.get_or_none(Invite.id == row.id)
        replay = _replay_user(current, request_id)
        if replay is not None:
            return _session_response(replay, 201)
        redeem_limiter.record_failure(ip, token_key)
        return _invite_invalid()
    except UserNameTaken:
        redeem_limiter.record_failure(ip, token_key)
        return _failure(
            "Ce nom d'utilisateur n'est pas disponible.",
            "username_unavailable",
            400,
            field="username",
        )
    except (WeakPassword, PasswordMismatch):
        redeem_limiter.record_failure(ip, token_key)
        return _failure(
            "Ce mot de passe est trop faible : 12 caractères minimum, pas un mot de passe courant.",
            "password_weak",
            400,
            field="password",
        )

    _remember_redeem(row.id, request_id, user.id)
    redeem_limiter.reset_user(token_key)
    logger.info("invite %s redeemed by user %s", row.id, user.id)
    from .. import onboarding as _onboarding

    _onboarding.warm_feed(current_app._get_current_object(), user)
    return _session_response(user, 201)


@invites_api.route("/admin/invites", methods=["GET"])
@login_required
def admin_invites_list():
    forbidden = _admin_guard()
    if forbidden is not None:
        return forbidden
    state_arg = request.args.get("state")
    if state_arg and state_arg not in STATES:
        return _failure("État inconnu.", "invalid_state", 400, field="state")
    try:
        limit = int(request.args.get("limit") or PAGE_DEFAULT)
    except (TypeError, ValueError):
        return _failure("Limite invalide.", "invalid_limit", 400, field="limit")
    limit = max(1, min(PAGE_MAX, limit))

    after_created = request.args.get("after_created")
    after_id = request.args.get("after_id")
    if bool(after_created) != bool(after_id):
        return _failure(
            "Curseur incomplet.", "invalid_cursor", 400, field="after_created"
        )
    after = None
    if after_created:
        if len(after_created) > 40:
            return _failure(
                "Curseur invalide.", "invalid_cursor", 400, field="after_created"
            )
        try:
            after = datetime.fromisoformat(after_created)
        except ValueError:
            return _failure(
                "Curseur invalide.", "invalid_cursor", 400, field="after_created"
            )
        if after.tzinfo is not None:
            return _failure(
                "Curseur invalide.", "invalid_cursor", 400, field="after_created"
            )
        if not invites.valid_id(after_id):
            return _failure(
                "Curseur invalide.", "invalid_cursor", 400, field="after_id"
            )

    at = now()
    query = Invite.select()
    if state_arg == "pending":
        query = query.where(
            Invite.used_at.is_null(True),
            Invite.revoked_at.is_null(True),
            Invite.expires > at,
        )
    elif state_arg == "used":
        query = query.where(Invite.used_at.is_null(False))
    elif state_arg == "revoked":
        query = query.where(
            Invite.used_at.is_null(True), Invite.revoked_at.is_null(False)
        )
    elif state_arg == "expired":
        query = query.where(
            Invite.used_at.is_null(True),
            Invite.revoked_at.is_null(True),
            Invite.expires <= at,
        )
    if after is not None:
        query = query.where(
            (Invite.created < after)
            | ((Invite.created == after) & (Invite.id < after_id))
        )
    rows = list(
        query.order_by(Invite.created.desc(), Invite.id.desc()).limit(limit + 1)
    )
    more = len(rows) > limit
    rows = rows[:limit]
    names = _names_for(rows)
    next_cursor = None
    if more and rows:
        next_cursor = {"created": _iso(rows[-1].created), "id": rows[-1].id}
    return jsonify(
        {"invites": [_row_json(row, names, at) for row in rows], "next": next_cursor}
    )


@invites_api.route("/admin/invites", methods=["POST"])
@login_required
def admin_invite_create():
    forbidden = _admin_guard()
    if forbidden is not None:
        return forbidden
    data, error = _json_body()
    if error is not None:
        return error
    note, error = _note(data)
    if error is not None:
        return error
    expires_in, error = _expires_in(data)
    if error is not None:
        return error
    allowed, retry_after = create_limiter.allow(f"admin:{request.webuser.id}")
    if not allowed:
        return _rate_limited(retry_after)

    at = now()
    try:
        with db.atomic():
            if invites.live_count(at) >= invites.MAX_LIVE:
                raise _LiveCap
            row, token = invites.create(request.webuser, note, expires_in, at)
            if invites.live_count(at) > invites.MAX_LIVE:
                raise _LiveCap
    except _LiveCap:
        return _live_cap()

    logger.info("admin %s created invite %s", request.webuser.name, row.id)
    resp = jsonify({"invite": _invite_json(row, at), "token": token})
    resp.status_code = 201
    return _no_store(resp)


@invites_api.route("/admin/invites/<invite_id>/revoke", methods=["POST"])
@login_required
def admin_invite_revoke(invite_id):
    forbidden = _admin_guard()
    if forbidden is not None:
        return forbidden
    row = _row_or_none(invite_id)
    if row is None:
        return _invite_not_found()
    if row.used_at is not None:
        return _invite_used()
    at = now()
    if invites.revoke(row.id, at) == 0:
        current = Invite.get_or_none(Invite.id == row.id)
        if current is None:
            return _invite_not_found()
        if current.used_at is not None:
            return _invite_used()
    logger.info("admin %s revoked invite %s", request.webuser.name, row.id)
    return jsonify({"revoked": True})


@invites_api.route("/admin/invites/<invite_id>/renew", methods=["POST"])
@login_required
def admin_invite_renew(invite_id):
    forbidden = _admin_guard()
    if forbidden is not None:
        return forbidden
    row = _row_or_none(invite_id)
    if row is None:
        return _invite_not_found()
    if row.used_at is not None:
        return _invite_used()
    allowed, retry_after = create_limiter.allow(f"admin:{request.webuser.id}")
    if not allowed:
        return _rate_limited(retry_after)

    ttl = int((row.expires - row.created).total_seconds())
    ttl = max(invites.TTL_MIN, min(invites.TTL_MAX, ttl))
    note = row.note
    at = now()
    try:
        with db.atomic():
            if invites.revoke(row.id, at) == 0:
                current = Invite.get_or_none(Invite.id == row.id)
                if current is None:
                    raise _Gone
                if current.used_at is not None:
                    raise _Used
            if invites.live_count(at) >= invites.MAX_LIVE:
                raise _LiveCap
            fresh, token = invites.create(request.webuser, note, ttl, at)
            if invites.live_count(at) > invites.MAX_LIVE:
                raise _LiveCap
    except _Gone:
        return _invite_not_found()
    except _Used:
        return _invite_used()
    except _LiveCap:
        return _live_cap()

    logger.info("admin %s renewed invite %s as %s", request.webuser.name, row.id, fresh.id)
    resp = jsonify({"invite": _invite_json(fresh, at), "token": token})
    resp.status_code = 201
    return _no_store(resp)
