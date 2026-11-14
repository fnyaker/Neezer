# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Single-use invitation tokens.

A token is ``<id>.<mac>``: a 22-character URL-safe id (16 random bytes) and a
24-character HMAC-SHA256 of ``"invite:" + id`` under the server's
``invite_secret``. The row stores only ``SHA-256(token)`` in ``token_hash``, so
a copy of the database opens nothing: the mac cannot be forged without the
secret and the hash cannot be reversed without the token.

``used_at`` is the source of truth for consumption (never ``used_by``, which
``ON DELETE SET NULL`` may clear). ``claim`` is one conditional ``UPDATE`` so
two simultaneous redeems have exactly one winner.
"""

from __future__ import annotations

import base64
import hashlib
import hmac
import re
import secrets
from datetime import timedelta

from .db import Invite, now
from .utils import get_secret_key

TTL_MIN = 3600
TTL_MAX = 30 * 24 * 3600
TTL_DEFAULT = 7 * 24 * 3600
NOTE_MAX = 64
MAX_LIVE = 200

_ID_RE = re.compile(r"\A[A-Za-z0-9_-]{22}\Z")
_TOKEN_RE = re.compile(r"\A([A-Za-z0-9_-]{22})\.([A-Za-z0-9_-]{24})\Z")
_DOMAIN = "invite:"


def _secret():
    key = get_secret_key("invite_secret")
    return key.encode("utf-8") if isinstance(key, str) else key


def _mac(invite_id):
    digest = hmac.new(
        _secret(), (_DOMAIN + invite_id).encode("ascii"), hashlib.sha256
    ).digest()
    return base64.urlsafe_b64encode(digest).rstrip(b"=").decode("ascii")[:24]


def valid_id(value):
    return isinstance(value, str) and _ID_RE.match(value) is not None


def new_id():
    return secrets.token_urlsafe(16)


def token_for(invite_id):
    return f"{invite_id}.{_mac(invite_id)}"


def hash_token(token):
    return hashlib.sha256(token.encode("utf-8")).hexdigest()


def parse_token(token):
    """The invite id a token names, or None when its shape or mac is wrong."""
    if not isinstance(token, str):
        return None
    match = _TOKEN_RE.match(token.strip())
    if match is None:
        return None
    invite_id, mac = match.groups()
    return invite_id if hmac.compare_digest(mac, _mac(invite_id)) else None


def find_any(token):
    """The row whose id and hash match this token, used or not, else None."""
    invite_id = parse_token(token)
    if invite_id is None:
        return None
    digest = hash_token(token.strip())
    row = Invite.get_or_none(Invite.id == invite_id)
    if row is None or not hmac.compare_digest(row.token_hash, digest):
        return None
    return row


def alive(row, at=None):
    if row is None:
        return False
    at = at or now()
    return row.revoked_at is None and row.used_at is None and row.expires > at


def find(token, at=None):
    row = find_any(token)
    return row if alive(row, at) else None


def state(row, at=None):
    """``used`` > ``revoked`` > ``expired`` > ``pending``."""
    at = at or now()
    if row.used_at is not None:
        return "used"
    if row.revoked_at is not None:
        return "revoked"
    return "expired" if row.expires <= at else "pending"


def live_count(at=None):
    at = at or now()
    return (
        Invite.select()
        .where(
            Invite.used_at.is_null(True),
            Invite.revoked_at.is_null(True),
            Invite.expires > at,
        )
        .count()
    )


def create(created_by, note=None, expires_in=TTL_DEFAULT, at=None):
    at = at or now()
    ttl = int(expires_in)
    if not TTL_MIN <= ttl <= TTL_MAX:
        raise ValueError("expires_in out of bounds")
    invite_id = new_id()
    token = token_for(invite_id)
    row = Invite.create(
        id=invite_id,
        token_hash=hash_token(token),
        created_by=created_by,
        created=at,
        expires=at + timedelta(seconds=ttl),
        note=note,
    )
    return row, token


def claim(invite_id, at):
    """Consume the invitation; 1 when this caller won, 0 when somebody else did."""
    return (
        Invite.update(used_at=at)
        .where(
            (Invite.id == invite_id)
            & Invite.used_at.is_null(True)
            & Invite.revoked_at.is_null(True)
            & (Invite.expires > at)
        )
        .execute()
    )


def mark_used_by(invite_id, user):
    return Invite.update(used_by=user.id).where(Invite.id == invite_id).execute()


def revoke(invite_id, at):
    return (
        Invite.update(revoked_at=at)
        .where(
            (Invite.id == invite_id)
            & Invite.used_at.is_null(True)
            & Invite.revoked_at.is_null(True)
        )
        .execute()
    )
