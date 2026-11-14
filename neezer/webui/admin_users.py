# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Administer the accounts: list, search, detail, role/state, delete.

Every route is ``@login_required`` then an admin check, so a non-admin gets the
same bare 403 whatever ``q`` or id was sent. The list is keyset, one statement
for the page and one for the total, and selects explicit columns so a password
hash is never even loaded. ``DELETE`` never deletes a file: it reassigns the
target's private uploads to the acting admin, revokes the target's pending
invitations, drops its provider grants, and refuses to remove the last active
admin, the caller, or the holder of the server's Deezer account.
"""

from __future__ import annotations

import json
import logging
import uuid

from flask import Blueprint, jsonify, request
from peewee import fn

from ..db import Playlist, Track, User
from ..managers.user import UserActionRefused, UserManager, fold_name
from ..utils import like_term

from . import _uuid_text, login_required, same_origin_guard

logger = logging.getLogger(__name__)

admin_users_api = Blueprint("admin_users_api", __name__)

DEFAULT_LIMIT = 50
MAX_LIMIT = 200
MAX_QUERY = 80
MAX_BODY = 8 * 1024

_COLUMNS = (
    User.id,
    User.name,
    User.mail,
    User.admin,
    User.jukebox,
    User.disabled,
    User.last_login,
    User.onboarding_step,
)

_REFUSALS = {
    "last_admin": (409, "Impossible : c'est le dernier administrateur actif."),
    "self_delete": (403, "Tu ne peux pas supprimer ton propre compte."),
    "self_disable": (403, "Tu ne peux pas désactiver ton propre compte."),
    "server_account_holder": (
        409,
        "Ce compte porte le compte Deezer du serveur. "
        "Changez-en le titulaire avant de le supprimer.",
    ),
}


@admin_users_api.before_request
def reject_cross_site():
    return same_origin_guard()


@admin_users_api.errorhandler(Exception)
def _json_error(exc):
    from werkzeug.exceptions import HTTPException

    if isinstance(exc, HTTPException):
        return jsonify({"error": exc.name}), exc.code
    logger.error("Unhandled error on %s %s", request.method, request.path, exc_info=True)
    return jsonify({"error": "server error"}), 500


def _admin_only():
    user = getattr(request, "webuser", None)
    if user is None or not user.admin:
        return jsonify({"error": "Réservé aux administrateurs.", "code": "forbidden"}), 403
    return None


def _bad(message="Paramètre invalide."):
    return jsonify({"error": message, "code": "invalid_request"}), 400


def _not_found():
    return jsonify({"error": "Compte introuvable.", "code": "not_found"}), 404


def _refused(exc):
    status, message = _REFUSALS.get(
        getattr(exc, "code", ""), (400, "Cette action est impossible.")
    )
    return jsonify({"error": message, "code": getattr(exc, "code", "refused")}), status


def _id(value):
    if not isinstance(value, str):
        return None
    try:
        return uuid.UUID(value)
    except (ValueError, TypeError, AttributeError):
        return None


def _payload(row):
    uid, name, mail, admin, jukebox, disabled, last_login, step = row
    return {
        "id": _uuid_text(uid),
        "name": name,
        "mail": mail or None,
        "admin": bool(admin),
        "jukebox": bool(jukebox),
        "disabled": bool(disabled),
        "last_login": last_login.isoformat() if last_login else None,
        "onboarding": step,
    }


def _from_user(user):
    return _payload(
        (
            user.id,
            user.name,
            user.mail,
            user.admin,
            user.jukebox,
            user.disabled,
            user.last_login,
            user.onboarding_step,
        )
    )


def _limit():
    raw = request.args.get("limit")
    if raw is None or raw == "":
        return DEFAULT_LIMIT, None
    try:
        limit = int(raw)
    except (TypeError, ValueError):
        return None, _bad("La limite doit être un entier entre 1 et 200.")
    if not 1 <= limit <= MAX_LIMIT:
        return None, _bad("La limite doit être un entier entre 1 et 200.")
    return limit, None


def _search_clause():
    raw = (request.args.get("q") or "").strip()
    if not raw:
        return None, None
    if len(raw) > MAX_QUERY:
        return None, _bad("La recherche est limitée à 80 caractères.")
    term = like_term(fold_name(raw), 1)
    if term is None:
        return None, _bad("La recherche ne contient aucun terme exploitable.")
    return (
        User.name_key.contains(term) | fn.LOWER(User.mail).contains(term)
    ), None


def _json_body():
    given = request.content_length
    if given is not None and given > MAX_BODY:
        return None, (
            jsonify({"error": "Requête trop volumineuse.", "code": "too_large"}),
            413,
        )
    if given is None:
        raw = request.stream.read(MAX_BODY + 1)
        if len(raw) > MAX_BODY:
            return None, (
                jsonify({"error": "Requête trop volumineuse.", "code": "too_large"}),
                413,
            )
        try:
            data = json.loads(raw)
        except (ValueError, UnicodeDecodeError):
            return None, _bad("Corps JSON attendu.")
    else:
        data = request.get_json(silent=True)
    if not isinstance(data, dict):
        return None, _bad("Corps JSON attendu.")
    return data, None


@admin_users_api.route("/admin/users")
@login_required
def admin_users_list():
    denied = _admin_only()
    if denied is not None:
        return denied
    limit, error = _limit()
    if error is not None:
        return error
    clause, error = _search_clause()
    if error is not None:
        return error

    counting = User.select(User.id)
    page = User.select(*_COLUMNS)
    if clause is not None:
        counting = counting.where(clause)
        page = page.where(clause)
    total = counting.count()

    after = request.args.get("after")
    if after:
        cursor = _id(after)
        if cursor is None:
            return _bad("Curseur invalide.")
        page = page.where(User.id > cursor)
    rows = list(page.order_by(User.id).limit(limit + 1).tuples())
    has_more = len(rows) > limit
    rows = rows[:limit]

    return jsonify(
        {
            "users": [_payload(row) for row in rows],
            "next": _payload(rows[-1])["id"] if has_more else None,
            "total": total,
        }
    )


@admin_users_api.route("/admin/users/<uid>")
@login_required
def admin_user_get(uid):
    denied = _admin_only()
    if denied is not None:
        return denied
    target = _id(uid)
    if target is None:
        return _not_found()
    user = User.get_or_none(User.id == target)
    if user is None:
        return _not_found()
    uploads = Track.select().where(Track.owner == user.id).count()
    playlists = Playlist.select().where(Playlist.user == user.id).count()
    return jsonify(
        {"user": _from_user(user), "uploads": uploads, "playlists": playlists}
    )


@admin_users_api.route("/admin/users/<uid>", methods=["PATCH"])
@login_required
def admin_user_patch(uid):
    denied = _admin_only()
    if denied is not None:
        return denied
    target = _id(uid)
    if target is None:
        return _not_found()
    data, error = _json_body()
    if error is not None:
        return error
    admin = data.get("admin")
    disabled = data.get("disabled")
    if "admin" not in data and "disabled" not in data:
        return _bad("Indiquez « admin » ou « disabled ».")
    for field, value in (("admin", admin), ("disabled", disabled)):
        if field in data and not isinstance(value, bool):
            return _bad(f"« {field} » doit être un booléen.")
    try:
        user = UserManager.update_guarded(
            target,
            request.webuser,
            admin=admin if "admin" in data else None,
            disabled=disabled if "disabled" in data else None,
        )
    except User.DoesNotExist:
        return _not_found()
    except UserActionRefused as exc:
        logger.info(
            "admin %s refused to change account %s (%s)",
            request.webuser.name,
            target,
            getattr(exc, "code", "refused"),
        )
        return _refused(exc)
    logger.info(
        "admin %s changed account %s (admin=%s disabled=%s)",
        request.webuser.name,
        user.name,
        user.admin,
        user.disabled,
    )
    return jsonify({"user": _from_user(user)})


@admin_users_api.route("/admin/users/<uid>", methods=["DELETE"])
@login_required
def admin_user_delete(uid):
    denied = _admin_only()
    if denied is not None:
        return denied
    target = _id(uid)
    if target is None:
        return _not_found()
    try:
        moved = UserManager.delete_guarded(target, request.webuser)
    except User.DoesNotExist:
        return _not_found()
    except UserActionRefused as exc:
        logger.info(
            "admin %s refused to delete account %s (%s)",
            request.webuser.name,
            target,
            getattr(exc, "code", "refused"),
        )
        return _refused(exc)
    return jsonify({"deleted": True, "uploads_reassigned": moved})
