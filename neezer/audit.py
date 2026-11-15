# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""A durable trace of administrative actions.

Written in the caller's transaction so a refused action leaves no row and a
successful one cannot be lost; never a secret, a token or a password — a short
phrase only. Reading is a plain query (``neezer-cli audit list``).
"""

from __future__ import annotations

import logging

from .db import AuditLog

logger = logging.getLogger(__name__)


def _text(value, limit):
    if value is None:
        return None
    text = " ".join(str(value).split())
    return text[:limit] or None


def record(action, actor=None, target=None, detail=None, result="ok"):
    actor_name = getattr(actor, "name", actor)
    try:
        return AuditLog.create(
            actor=_text(actor_name, 64),
            action=_text(action, 64) or "unknown",
            target=_text(target, 128),
            detail=_text(detail, 200),
            result=_text(result, 16) or "ok",
        )
    except Exception:
        logger.warning("could not record the audit row for %s", action, exc_info=True)
        return None
