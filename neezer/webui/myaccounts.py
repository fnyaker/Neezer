# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Réglages → Sources → *Mes comptes*: each person's own accounts, and what
they bring into that person's library (the engine is neezer/libraryimport.py).

- ``GET /me/accounts``: my accounts (the server's Deezer account too, for
  whoever holds it), what each imports automatically and how the last import
  went. A YouTube Music session linked for the export is an account here.
- ``POST /me/accounts/deezer``: link my own Deezer with an ARL (typed, or read
  from Deezer's own login page by the Android app). Checked by signing in.
- ``DELETE /me/accounts/<id>``: unlink it (nothing imported is removed).
- ``PUT /me/accounts/<id>``: what is imported automatically.
- ``PUT /me/accounts/default``: my default platform, the ONE whose favourites
  are synced every day (``{"account": id}``, or null for none). Choosing one
  imports from it at once.
- ``POST /me/accounts/<id>/import``: import what is new now.
- ``GET|PUT /me/accounts/<id>/library/<kind>``: the account's artists, albums
  or playlists, to tick or untick one by one.

Every credential stays the holder's: no route here names or returns one, and
none is open to a remote-control session (``remote.POLICY``: NEVER).

The onboarding's account step links through ``link_deezer`` and
``link_youtube_headers`` rather than through its own copy, so the server-account
guards and the encryption exist once.
"""

from __future__ import annotations

import hashlib
import hmac
import json
import logging
import uuid

from flask import current_app, jsonify, request

from .. import access, libraryimport as li
from ..db import Meta, ProviderAccount, ServiceLink, db, now
from ..providers.errors import ProviderAuth, ProviderError, ProviderUnavailable
from ..ratelimit import WindowLimiter
from . import login_required, webapi

logger = logging.getLogger(__name__)
DEEZER_LINK = WindowLimiter(10, 900)

ARL_MAX = 512
NAMES = {"deezer": "Deezer", "youtube": "YouTube Music"}


def _me():
    return request.webuser


def _row(account_id):
    """My account of that id, or None (somebody else's is as good as absent)."""
    try:
        uuid.UUID(str(account_id))
    except ValueError:
        return None
    row = ProviderAccount.get_or_none(ProviderAccount.id == account_id)
    if row is None or row.user_id is None or str(row.user_id) != str(_me().id):
        return None
    if row.provider not in li.SOURCES:
        return None
    return row


def link_youtube(user_id, label=None):
    """The YouTube Music session linked for the export is also an account to
    import from: its row, made or refreshed. Returns (row, created)."""
    row = ProviderAccount.get_or_none(
        (ProviderAccount.provider == "youtube") & (ProviderAccount.user == user_id)
    )
    created = row is None
    if created:
        row = ProviderAccount.create(
            provider="youtube", user=user_id, label=(label or "YouTube Music")[:64],
            mode="headers", status="ok", checked=now(), created=now(),
        )
        li.keep_default(user_id, row.id)
    else:
        row.status, row.checked = "ok", now()
        if label:
            row.label = label[:64]
        row.save()
    access.bump()
    return row, created


def link_youtube_headers(user, raw):
    """Link ``user``'s YouTube Music from the headers of a signed-in ``browse``
    request: verified, encrypted at rest, then imported. Returns
    ``(status, payload)`` like :func:`link_deezer`; no header reaches a log."""
    from ..services import ServiceError, ytmusic
    from . import services as web_services

    try:
        headers_json = ytmusic.parse_headers(raw)
        account = ytmusic.Client(headers_json).account()
    except ServiceError as exc:
        return 400, {"error": str(exc), "code": "invalid_headers"}
    web_services._store(user.id, "youtube", headers_json, account)
    row, _created = link_youtube(user.id, account)
    li.forget_client(row.id)
    li.start(current_app._get_current_object(), row.id)
    return 200, {"account": _account_json(row, _default_id(user))}


def unlink(row):
    li.forget_account(row.id)
    access.forget_account(row.id)
    if row.provider == "youtube":
        ServiceLink.delete().where(
            (ServiceLink.user == row.user_id) & (ServiceLink.service == "youtube")
        ).execute()
    row.delete_instance()
    access.bump()


def unlink_youtube(user_id):
    row = ProviderAccount.get_or_none(
        (ProviderAccount.provider == "youtube") & (ProviderAccount.user == user_id)
    )
    if row is not None:
        unlink(row)


def _account_json(row, default_id=None):
    opts = li.options(row)
    live = li.status(row.id)
    return {
        "id": str(row.id),
        "provider": row.provider,
        "name": NAMES.get(row.provider, row.provider),
        "label": row.label,
        "mode": row.mode,
        "server": row.mode == access.SERVER,
        "status": row.status,
        "auto": opts["auto"],
        "last": opts["last"],
        "lossless": opts["lossless"],
        "running": live,
        "default": default_id is not None and str(default_id) == str(row.id),
        "kinds": list(li.KINDS),
        "pickable": list(li.PICKABLE),
    }


def _safe_account_json(row, default_id):
    """One unreadable account is that account's problem, not the page's: it is
    listed with its cause, and the others stay usable."""
    try:
        return _account_json(row, default_id)
    except Exception as exc:
        logger.exception("account %s unreadable", row.id)
        return {
            "id": str(row.id),
            "provider": row.provider,
            "name": NAMES.get(row.provider, row.provider),
            "label": row.label,
            "mode": row.mode,
            "server": row.mode == access.SERVER,
            "status": "broken",
            # The KIND of failure only: an exception's own text can carry a
            # path or a connection string, and this is shown in a browser.
            # The traceback is in the server's log, where it belongs.
            "error": type(exc).__name__,
            "auto": {},
            "last": None,
            "lossless": None,
            "running": None,
            "default": default_id is not None and str(default_id) == str(row.id),
            "kinds": [],
            "pickable": [],
        }


@webapi.route("/me/accounts")
@login_required
def my_accounts():
    me = _me()
    if (
        ServiceLink.get_or_none((ServiceLink.user == me.id) & (ServiceLink.service == "youtube"))
        is not None
        and ProviderAccount.get_or_none(
            (ProviderAccount.provider == "youtube") & (ProviderAccount.user == me.id)
        )
        is None
    ):
        # Linked for the export before accounts existed: it is one now.
        link = ServiceLink.get((ServiceLink.user == me.id) & (ServiceLink.service == "youtube"))
        link_youtube(me.id, link.account)
    return my_accounts_payload()


def my_accounts_payload(**extra):
    me = _me()
    rows = li._rows_of(me.id)
    holds_server = any(r.mode == access.SERVER for r in rows)
    default = li._default_of(rows)
    default_id = default.id if default is not None else None
    return jsonify(
        {
            "accounts": [_safe_account_json(r, default_id) for r in rows],
            "default": str(default_id) if default_id is not None else None,
            # A personal Deezer next to the server's is one account too many:
            # whoever holds the server's imports from it.
            "can_link": {"deezer": not holds_server},
            **extra,
        }
    )


def _deezer_login(arl):
    """Sign in with ``arl`` once: ``{"id", "name"}`` of the account, or raises
    ProviderAuth (refused) / ProviderUnavailable (Deezer not answering)."""
    from deezerpy.errors import is_transport_failure

    from ..deezer.provider import DeezerProvider

    cfg = current_app.config.get("DEEZER") or {}
    probe = DeezerProvider(arl, cfg.get("archive_dir") or "", cfg.get("quality") or "FLAC")
    try:
        user = probe.dz.current_user or {}
        return {
            "id": str(user.get("id") or ""),
            "name": str(user.get("name") or "Deezer"),
            # Lot P3: an account that cannot stream FLAC never fetches the sound.
            "lossless": bool(user.get("can_stream_lossless")),
        }
    except Exception as exc:
        login = getattr(probe, "_login_error", None)
        if login and login[0] == "arl":
            raise ProviderAuth(login[1], provider="deezer") from exc
        if is_transport_failure(exc) or (login and login[0] == "network"):
            raise ProviderUnavailable(str(exc), provider="deezer") from exc
        raise ProviderError(str(exc), provider="deezer") from exc
    finally:
        dz = getattr(probe, "_dz", None)
        if dz is not None:
            try:
                dz.close()
            except Exception:
                logger.debug("closing a probe session failed", exc_info=True)


SERVER_IDENTITY_KEY = "dz.server_identity"


def _server_arl_digest():
    client = getattr(current_app, "deezer", None)
    arl = getattr(client, "arl", None) if client is not None else None
    return hashlib.sha256(arl.encode("utf-8")).hexdigest() if arl else ""


def _remember_server_deezer_id(uid):
    digest = _server_arl_digest()
    if not uid or not digest:
        return
    value = json.dumps({"id": str(uid), "arl_sha": digest})
    try:
        with db.atomic():
            Meta.insert(key=SERVER_IDENTITY_KEY, value=value).on_conflict(
                conflict_target=[Meta.key], update={Meta.value: value}
            ).execute()
    except Exception:
        logging.getLogger(__name__).debug("could not remember the server Deezer id", exc_info=True)


def _server_deezer_user_id():
    client = getattr(current_app, "deezer", None)
    dz = getattr(client, "_dz", None) if client is not None else None
    if dz is not None:
        try:
            uid = str((dz.current_user or {}).get("id") or "")
        except Exception:
            uid = ""
        if uid:
            _remember_server_deezer_id(uid)
            return uid
    digest = _server_arl_digest()
    if not digest:
        return ""
    try:
        row = Meta.get_or_none(Meta.key == SERVER_IDENTITY_KEY)
        saved = json.loads(row.value) if row is not None and row.value else {}
    except Exception:
        return ""
    if saved.get("arl_sha") != digest:
        return ""
    return str(saved.get("id") or "")


def link_deezer(user, data):
    """Link ``user``'s own Deezer with an ARL (typed, or read from Deezer's own
    login page by the Android app). ``(status, payload)``: the account, or the
    cause in words. Only the failure's kind is logged, never the credential."""
    from ..managers.user import encrypt_password

    if not isinstance(data, dict):
        data = {}
    arl = "".join(str(data.get("arl") or "").split())
    if not arl or len(arl) > ARL_MAX or not arl.isalnum():
        return 400, {"error": "Cet ARL n'a pas la forme d'un ARL.", "code": "invalid_arl"}
    server_arl = getattr(getattr(current_app, "deezer", None), "arl", None)
    if server_arl and hmac.compare_digest(arl, server_arl):
        return 409, {
            "error": "C'est le compte Deezer du serveur : demandez à son titulaire de vous le prêter (Sources).",
            "code": "server_account",
        }
    row = ProviderAccount.get_or_none(
        (ProviderAccount.provider == "deezer") & (ProviderAccount.user == user.id)
    )
    if row is not None and row.mode == access.SERVER:
        return 409, {
            "error": "Votre compte Deezer est celui du serveur : son ARL se change dans Compte Deezer.",
            "code": "server_account_holder",
        }
    try:
        who = _deezer_login(arl)
    except ProviderAuth:
        return 400, {"error": "Deezer refuse cet ARL (expiré, révoqué ou mal copié).", "code": "provider_refused"}
    except ProviderUnavailable:
        return 503, {"error": "Deezer ne répond pas : réessayez dans un moment.", "code": "provider_unreachable"}
    except ProviderError as exc:
        logger.warning("linking a Deezer account failed: %s", type(exc).__name__)
        return 502, {
            "error": "Deezer a refusé la connexion pour une autre raison : réessayez plus tard.",
            "code": "provider_error",
        }
    if who["id"] and who["id"] == _server_deezer_user_id():
        return 409, {
            "error": "C'est le compte Deezer du serveur : demandez à son titulaire de vous le prêter (Sources).",
            "code": "server_account",
        }
    blob = encrypt_password(arl)
    mode = "browser" if data.get("mode") == "browser" else "arl"
    if row is None:
        row = ProviderAccount.create(
            provider="deezer", user=user.id, label=who["name"][:64], secret=blob, mode=mode,
            status="ok", checked=now(), created=now(),
        )
    else:
        li.forget_client(row.id)
        row.secret, row.label, row.mode, row.status, row.checked = blob, who["name"][:64], mode, "ok", now()
        row.save()
    opts = li.options(row)
    opts["lossless"] = who.get("lossless", False)
    li._save_options(row, opts)
    li.keep_default(user.id, row.id)
    access.bump()
    default = li.default_account(user.id)
    if default is not None and default.id == row.id:
        # Only the default platform syncs the favourites (and a first account is
        # the default until its holder chooses).
        li.start(current_app._get_current_object(), row.id)
    return 200, {"account": _account_json(row, default.id if default is not None else None)}


@webapi.route("/me/accounts/deezer", methods=["POST"])
@login_required
def my_accounts_link_deezer():
    user = _me()
    allowed, retry = DEEZER_LINK.allow(f"deezer-link:{user.id}")
    if not allowed:
        response = jsonify(
            {
                "error": "Trop de tentatives de connexion Deezer : réessayez dans un moment.",
                "code": "rate_limited",
            }
        )
        response.status_code = 429
        response.headers["Retry-After"] = str(retry)
        return response
    status, payload = link_deezer(user, request.get_json(silent=True) or {})
    return jsonify(payload), status


@webapi.route("/me/accounts/<account_id>", methods=["DELETE"])
@login_required
def my_accounts_unlink(account_id):
    row = _row(account_id)
    if row is None:
        return jsonify({"error": "compte inconnu"}), 404
    if row.mode == access.SERVER:
        return jsonify({"error": "Le compte du serveur se règle dans Compte Deezer."}), 409
    unlink(row)
    return jsonify({"ok": True})


@webapi.route("/me/accounts/<account_id>", methods=["PUT"])
@login_required
def my_accounts_options(account_id):
    row = _row(account_id)
    if row is None:
        return jsonify({"error": "compte inconnu"}), 404
    auto = (request.get_json(silent=True) or {}).get("auto")
    if not isinstance(auto, dict) or any(k not in li.KINDS for k in auto):
        return jsonify({"error": "auto invalide"}), 400
    li.set_auto(row, auto)
    return jsonify({"account": _account_json(row, _default_id())})


def _default_id(user=None):
    default = li.default_account((user or _me()).id)
    return default.id if default is not None else None


@webapi.route("/me/accounts/default", methods=["PUT"])
@login_required
def my_accounts_default():
    data = request.get_json(silent=True) or {}
    account = data.get("account")
    if account is not None and _row(account) is None:
        return jsonify({"error": "compte inconnu"}), 404
    started = li.set_default(current_app._get_current_object(), _me().id, account)
    return my_accounts_payload(started=started)


@webapi.route("/me/accounts/<account_id>/import", methods=["POST"])
@login_required
def my_accounts_import(account_id):
    row = _row(account_id)
    if row is None:
        return jsonify({"error": "compte inconnu"}), 404
    kinds = (request.get_json(silent=True) or {}).get("kinds")
    if kinds is not None and (not isinstance(kinds, list) or any(k not in li.KINDS for k in kinds)):
        return jsonify({"error": "kinds invalide"}), 400
    started = li.start(current_app._get_current_object(), row.id, kinds=kinds or None)
    row = ProviderAccount.get_by_id(row.id)
    return jsonify({"started": started, "account": _account_json(row, _default_id())}), 202


def _provider_error(exc):
    # 424, not 503: the web app retries a 5xx GET three times, and the
    # provider's own calls have already retried within their budget.
    if isinstance(exc, ProviderAuth):
        return jsonify({"error": "Le compte refuse la connexion : reliez-le à nouveau.", "reason": "auth"}), 409
    if isinstance(exc, ProviderUnavailable):
        return jsonify({"error": "Le service ne répond pas : réessayez dans un moment.", "reason": "network"}), 424
    logger.warning("reading an account's library failed: %r", exc)
    return jsonify({"error": "Le service a répondu par une erreur : réessayez plus tard.", "reason": "error"}), 502


@webapi.route("/me/accounts/<account_id>/library/<kind>")
@login_required
def my_accounts_library(account_id, kind):
    row = _row(account_id)
    if row is None or kind not in li.PICKABLE:
        return jsonify({"error": "introuvable"}), 404
    try:
        items = li.listing(current_app._get_current_object(), row, kind, fresh=request.args.get("fresh") == "1")
    except ProviderError as exc:
        return _provider_error(exc)
    return jsonify({"items": items})


@webapi.route("/me/accounts/<account_id>/library/<kind>", methods=["PUT"])
@login_required
def my_accounts_library_set(account_id, kind):
    row = _row(account_id)
    if row is None or kind not in li.PICKABLE:
        return jsonify({"error": "introuvable"}), 404
    data = request.get_json(silent=True) or {}
    refs = data.get("refs")
    if refs is None and data.get("ref") is not None:
        refs = [data.get("ref")]
    if (
        not isinstance(refs, list)
        or not refs
        or len(refs) > li.MAX_ITEMS
        or any(not isinstance(r, (str, int)) or not str(r) or len(str(r)) > 64 for r in refs)
        or not isinstance(data.get("on"), bool)
    ):
        return jsonify({"error": "refs (ou ref) et on requis"}), 400
    return jsonify(li.set_items(current_app._get_current_object(), row, kind, refs, data["on"]))
