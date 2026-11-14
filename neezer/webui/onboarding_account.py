# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""The account step of the onboarding: what a person can link, and the two
writes that bring something in.

- ``GET /onboarding/account/options``: what is actually possible for this
  person right now — Deezer (linked or not, ARL or the Android native login),
  YouTube Music (installed, switched on, linked), a public playlist link for
  Deezer, Spotify, Apple Music and YouTube, and the import already running.
  No secret, no token, no password: a label at most. Answered 200 with Deezer
  switched off, so the screen always paints.
- ``POST /onboarding/account/link``: link the caller's own Deezer (ARL) or
  YouTube Music (the headers of a signed-in ``browse`` request), through the
  very engine ``myaccounts`` uses. Body ≤ 8 KiB, same-origin, rate limited,
  and the account is the caller's alone.
- ``POST /onboarding/account/import``: read a public playlist link into a
  background job (``playlist_import``, SSRF-checked by ``linkimport``) and
  answer its id; ``GET /import/playlist/<id>`` carries the progress, its phase
  and the cause of a failure. Keeping it is the member save path: a local
  playlist and no bulk archiving.

Nothing here writes to a provider, downloads a discography, or blocks the
step: every card is optional and an error never stops ``account/done``.
"""

from __future__ import annotations

import json
import logging

from flask import Blueprint, current_app, jsonify, request

from .. import access, linkimport
from .. import libraryimport as li
from ..db import ProviderAccount
from ..deezer import onboarding_pool as pool
from ..ratelimit import WindowLimiter
from . import login_required, same_origin_guard
from . import myaccounts, playlist_import
from .remote import NEVER, POLICY

logger = logging.getLogger(__name__)

MAX_BODY = 8 * 1024
LINK_MAX = 10
LINK_WINDOW = 900

link_limiter = WindowLimiter(LINK_MAX, LINK_WINDOW)

ONBOARDING_ACCOUNT_POLICY = {
    "onboarding_account_options": NEVER,
    "onboarding_account_link": NEVER,
    "onboarding_account_import": NEVER,
}

STEPS = ("artists", "account", "done")


class _OnboardingAccountBlueprint(Blueprint):
    def register(self, app, options):
        super().register(app, options)
        POLICY.update(ONBOARDING_ACCOUNT_POLICY)


onboarding_account_api = _OnboardingAccountBlueprint("onboarding_account_api", __name__)


@onboarding_account_api.before_request
def reject_cross_site():
    return same_origin_guard()


@onboarding_account_api.errorhandler(Exception)
def _json_error(exc):
    from werkzeug.exceptions import HTTPException

    if isinstance(exc, HTTPException):
        if exc.code == 413:
            return jsonify({"error": "Requête trop volumineuse.", "code": "too_large"}), 413
        return jsonify({"error": exc.name}), exc.code
    logger.error("Unhandled error on %s %s", request.method, request.path, exc_info=True)
    return jsonify({"error": "server error"}), 500


def _failure(status, code, message, retry_after=None):
    response = jsonify({"error": message, "code": code})
    response.status_code = status
    if retry_after is not None:
        response.headers["Retry-After"] = str(max(1, int(retry_after)))
    return response


def _json_body():
    length = request.content_length
    if length is not None and length > MAX_BODY:
        return None, _failure(413, "too_large", "Requête trop volumineuse.")
    if not request.is_json:
        return None, _failure(400, "invalid_body", "Requête invalide.")
    if length is None:
        try:
            raw = request.stream.read(MAX_BODY + 1)
        except Exception:
            return None, _failure(400, "invalid_body", "Requête invalide.")
        if len(raw) > MAX_BODY:
            return None, _failure(413, "too_large", "Requête trop volumineuse.")
        if not raw.strip():
            return None, _failure(400, "invalid_body", "Requête invalide.")
        try:
            data = json.loads(raw)
        except (ValueError, UnicodeDecodeError):
            return None, _failure(400, "invalid_body", "Requête invalide.")
    else:
        data = request.get_json(silent=True)
    if not isinstance(data, dict):
        return None, _failure(400, "invalid_body", "Requête invalide.")
    return data, None


def _step(user) -> str:
    value = getattr(user, "onboarding_step", None) or "done"
    return value if value in STEPS else "done"


def _linked(rows, provider):
    return next((row for row in rows if row.provider == provider), None)


def _account_entry(row):
    if row is None:
        return None
    opts = li.options(row)
    return {
        "id": str(row.id),
        "label": row.label,
        "mode": row.mode,
        "server": row.mode == access.SERVER,
        "status": row.status,
        "auto": opts["auto"],
        "last": opts["last"],
        "running": li.status(row.id),
    }


def _import_entry(source, label, available, reason):
    return {
        "source": source,
        "label": label,
        "mode": "public_link",
        "available": bool(available),
        "reason": reason,
    }


def account_options(user):
    """What this person can do at the account step. One query for their
    accounts (no credential column is even read), then memory (``access`` is
    cached): no provider is asked."""
    app = current_app._get_current_object()
    rows = list(
        ProviderAccount.select(
            ProviderAccount.id,
            ProviderAccount.provider,
            ProviderAccount.label,
            ProviderAccount.mode,
            ProviderAccount.status,
            ProviderAccount.options,
            ProviderAccount.created,
        )
        .where(ProviderAccount.user == user)
        .order_by(ProviderAccount.created, ProviderAccount.provider)
    )
    deezer = _linked(rows, "deezer")
    youtube = _linked(rows, "youtube")
    deezer_up = pool.deezer_enabled(app)
    dz_links = bool(deezer_up and access.can(user, "deezer", "links"))
    dz_server = deezer is not None and deezer.mode == access.SERVER
    yt = pool.youtube_status(user)
    yt_up = bool(yt["enabled"])
    yt_reason = yt["error"] if yt["error"] in ("absent", "disabled") else None
    imports = [
        _import_entry(
            "deezer",
            "Deezer",
            dz_links,
            None if dz_links else ("deezer_denied" if deezer_up else "deezer_disabled"),
        ),
        _import_entry("spotify", "Spotify", True, None),
        _import_entry("apple", "Apple Music", True, None),
        _import_entry("youtube", "YouTube Music", yt_up, yt_reason),
    ]
    return {
        "step": _step(user),
        "deezer": {
            "available": deezer_up,
            "can_link": bool(deezer_up and not dz_server),
            "linked": deezer is not None,
            "account": _account_entry(deezer),
            "auth_modes": ["arl", "native"] if deezer_up else [],
            "reason": None if deezer_up else "deezer_disabled",
        },
        "youtube": {
            "available": yt_up,
            "can_link": yt_up,
            "installed": bool(yt["installed"]),
            "enabled": yt_up,
            "denied": bool(yt["denied"]),
            "desktop": True,
            "linked": youtube is not None,
            "account": _account_entry(youtube),
            "auth_modes": ["headers"] if yt_up else [],
            "reason": yt_reason,
        },
        "imports": imports,
        "can_save_imports": True,
        "job": playlist_import.newest_running(user.id),
    }


@onboarding_account_api.route("/onboarding/account/options")
@login_required
def onboarding_account_options():
    return jsonify(account_options(request.webuser))


@onboarding_account_api.route("/onboarding/account/link", methods=["POST"])
@login_required
def onboarding_account_link():
    user = request.webuser
    data, failure = _json_body()
    if failure is not None:
        return failure
    provider = data.get("provider")
    if provider not in ("deezer", "youtube"):
        return _failure(400, "invalid_provider", "Choisissez Deezer ou YouTube Music.")
    if provider == "deezer" and not pool.deezer_enabled(current_app._get_current_object()):
        return _failure(400, "deezer_disabled", "Deezer n'est pas configuré sur ce serveur.")
    if provider == "youtube":
        status = pool.youtube_status(user)
        if not status["enabled"]:
            reason = status["error"] if status["error"] in ("absent", "disabled") else "unavailable"
            return _failure(
                400, reason, "YouTube Music n'est pas disponible sur ce serveur."
            )
    allowed, retry_after = link_limiter.allow(f"u:{user.id}")
    if not allowed:
        return _failure(
            429,
            "rate_limited",
            "Trop de tentatives. Réessaie dans quelques minutes.",
            retry_after,
        )
    if provider == "deezer":
        status, payload = myaccounts.link_deezer(user, data)
    else:
        status, payload = myaccounts.link_youtube_headers(user, data.get("headers"))
    if status >= 400:
        return _failure(
            status,
            payload.get("code") or "link_refused",
            payload.get("error") or "La liaison a échoué.",
            retry_after=30 if status == 503 else None,
        )
    return jsonify(
        {
            "provider": provider,
            "account": payload.get("account"),
            "options": account_options(user),
        }
    )


@onboarding_account_api.route("/onboarding/account/import", methods=["POST"])
@login_required
def onboarding_account_import():
    user = request.webuser
    data, failure = _json_body()
    if failure is not None:
        return failure
    text = data.get("url")
    if not isinstance(text, str) or not text.strip():
        return _failure(400, "missing_url", "Colle le lien d'une playlist.")
    link = linkimport.parse(text)
    if link is None:
        return _failure(
            400,
            "invalid_link",
            "Lien non reconnu : colle le lien d'une playlist Deezer, Spotify, YouTube ou Apple Music",
        )
    if link["source"] == "deezer" and not access.can(user, "deezer", "links"):
        return _failure(
            403, "deezer_denied", "Vous n'avez pas accès aux liens Deezer sur ce serveur."
        )
    status, payload = playlist_import.start_link(text, user)
    if status != 200:
        return _failure(
            status,
            payload.get("code") or "import_refused",
            payload.get("error") or "L'import n'a pas pu démarrer.",
            retry_after=5 if status == 429 else None,
        )
    job = playlist_import.snapshot(payload["id"], user.id)
    return jsonify({"id": payload["id"], "job": job, "options": account_options(user)}), 202
