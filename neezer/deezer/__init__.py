# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Deezer proxy layer: virtual library + archive-on-play streaming."""

import logging
import re

from .provider import DeezerProvider, DeezerError

__all__ = [
    "DeezerProvider",
    "DeezerError",
    "get_provider",
    "stored_arl",
    "store_arl",
    "valid_arl",
    "ARL_META_KEY",
]

logger = logging.getLogger(__name__)

# The admin-set ARL OVERRIDES the one from the config file / docker-compose
# environment: a credential that expires every few months has to be replaceable
# without editing a file and restarting the container. It is an encrypted
# ``server-arl:`` blob in the secret column of the ``mode=server`` provider
# account, the same place every other credential is kept. ``ARL_META_KEY`` is the
# plaintext row earlier releases wrote; it is read once, re-encrypted and removed.
ARL_META_KEY = "deezer_arl"
_SERVER_ARL_PREFIX = "server-arl:"
_SERVER_LABEL = "Compte du serveur"

# An ARL is an opaque session cookie value: a long run of hex. Validated before
# it is ever stored or sent, because it goes out as a Cookie header — a stray
# newline or semicolon there is header injection, and the surrounding code has
# no business trusting an admin-pasted string blindly.
_ARL_RE = re.compile(r"\A[0-9A-Za-z._-]{32,255}\Z")


def valid_arl(value) -> bool:
    """Whether `value` is shaped like a Deezer ARL (safe to store and send)."""
    return bool(value and _ARL_RE.fullmatch(str(value).strip()))


def _server_row():
    from ..db import ProviderAccount

    return (
        ProviderAccount.select()
        .where(
            (ProviderAccount.provider == "deezer")
            & (ProviderAccount.mode == "server")
        )
        .first()
    )


def _server_blob(row) -> str:
    blob = (row.secret or "").strip() if row is not None else ""
    return blob[len(_SERVER_ARL_PREFIX) :] if blob.startswith(_SERVER_ARL_PREFIX) else ""


def _decrypt_server_arl(blob: str) -> "str | None":
    from ..managers.user import decrypt_password

    try:
        value = decrypt_password(blob).strip()
    except Exception:
        logger.warning("Could not decrypt the stored Deezer ARL")
        return None
    if value and not valid_arl(value):
        logger.warning("Ignoring a malformed stored ARL")
        return None
    return value or None


def _drop_legacy_arl():
    from ..db import Meta

    try:
        Meta.delete().where(Meta.key == ARL_META_KEY).execute()
    except Exception:
        logger.debug("Could not remove the legacy stored ARL", exc_info=True)


def stored_arl() -> "str | None":
    """The admin-set ARL from the database, or None when there isn't one."""
    from ..db import Meta

    try:
        row = _server_row()
        blob = _server_blob(row)
        legacy = Meta.get_or_none(Meta.key == ARL_META_KEY)
    except Exception:  # DB not ready (very early boot) — fall back to config
        logger.debug("Could not read the stored ARL", exc_info=True)
        return None
    if blob:
        value = _decrypt_server_arl(blob)
        if value:
            if legacy is not None:
                _drop_legacy_arl()
            return value
    if legacy is None:
        return None
    value = (legacy.value or "").strip()
    if not value:
        _drop_legacy_arl()
        return None
    if not valid_arl(value):
        logger.warning("Ignoring a malformed stored ARL")
        return None
    try:
        store_arl(value)
    except Exception:
        logger.warning("Could not migrate the stored Deezer ARL", exc_info=True)
    return value


def store_arl(value) -> None:
    """Persist (or clear, with a falsy value) the admin-set ARL."""
    from ..db import Meta, ProviderAccount, db, now
    from ..managers.user import encrypt_password

    value = (value or "").strip()
    if value and not valid_arl(value):
        raise ValueError("malformed ARL")
    blob = _SERVER_ARL_PREFIX + encrypt_password(value) if value else None
    with db.atomic():
        row = _server_row()
        if row is None:
            if value:
                ProviderAccount.create(
                    provider="deezer",
                    user=None,
                    label=_SERVER_LABEL,
                    secret=blob,
                    mode="server",
                    status="ok",
                    created=now(),
                )
        else:
            if value:
                row.secret = blob
                row.save()
            elif _server_blob(row):
                row.secret = None
                row.save()
                if row.user_id is None:
                    from ..access import forget_account

                    forget_account(row.id)
                    row.delete_instance()
        Meta.delete().where(Meta.key == ARL_META_KEY).execute()


def _section(config) -> dict:
    if isinstance(config, dict):
        return config.get("DEEZER", config)
    if hasattr(config, "DEEZER"):
        return config.DEEZER
    return config["DEEZER"]


def get_provider(config) -> "DeezerProvider | None":
    """Build a provider from a neezer config object, Flask config, or dict.

    Accepts either the whole config (``.DEEZER`` / ``["DEEZER"]``) or the
    ``DEEZER`` section dict directly. A stored (admin-set) ARL wins over the
    configured one, and is itself enough to turn the proxy on — saving an ARL in
    the admin UI is as explicit an "enable this" as setting it in the config.
    """
    cfg = _section(config)
    override = stored_arl()
    if override:
        cfg = dict(cfg or {})
        cfg["arl"] = override
        cfg["enabled"] = True
        logger.info("Using the admin-set Deezer ARL from the database")
    return DeezerProvider.from_config(cfg)
