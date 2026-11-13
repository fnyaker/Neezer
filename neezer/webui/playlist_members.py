# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Collaborative playlists: who is on one, and the two tiny read probes.

Every route rides the shared ``webapi`` blueprint (imported at the bottom of
``webui/__init__.py``, the ``playlist_share.py`` pattern), so it inherits the
session guard and the same-origin guard and ``neezer/web.py`` is untouched.

The matrix lives in ``neezer/playlist_access.py`` — this module only applies it
on the wire:

* ``GET  /api/playlist/<id>/version`` — the 30 s poll's answer. Counters and
  the author hint only (a few hundred bytes), never the tracklist; a public
  reader gets it without the two ``last_edited_*`` fields.
* ``GET  /api/playlist/<id>/members`` — the collaborators, the caller's role
  and capabilities. 404 for a public reader (a public link carries no member
  names).
* ``POST /api/playlist/<id>/members`` — invite by EXACT account name.
* ``PATCH /api/playlist/<id>/members/<user_id>`` — editor or viewer.
* ``DELETE /api/playlist/<id>/members/<user_id>`` — remove (idempotent).
* ``POST /api/playlist/<id>/leave`` — an editor/viewer leaves.

Rules, all enforced here:

* non-member = **404** on every route (never an existence oracle); a member
  lacking the right = **403**;
* invite/role/remove are owner or admin only — never an editor;
* no account directory: the body carries one exact username, an unknown one is
  a plain ``404 {"error":"unknown user"}``; self is 400, the owner is 409;
* every membership mutation is one transaction: bump ``members_version``
  (the row lock), mutate, list. A stale ``members_version`` is a 409 whose body
  carries both counters and ``scope: "members"``, never the member list;
* caps: 100 members per playlist, and the edge lot's ``WindowLimiter`` (30
  mutations / 10 min / account) when installed — a 429 with ``Retry-After``;
* the role a route accepts is only ``editor`` or ``viewer``: ``owner`` is never
  assignable, and the owner is decided by ``Playlist.user_id`` first.
"""

from __future__ import annotations

import uuid

from flask import jsonify, request

from ..db import PlaylistMember, User, db, now
from .. import playlist_access as placcess

from . import (
    PLAYLIST_BODY_METADATA,
    _playlist_body,
    _playlist_conflict,
    _playlist_expected,
    _playlist_guard,
    _rate_limited,
    _visible_playlist_count,
    _window_limiter,
    login_required,
    webapi,
)

#: A playlist is a social object, not a mailing list. With the owner included.
PLAYLIST_MAX_MEMBERS = 100
#: Membership mutations per account per 10 minutes (the plan's 30/10 min).
MEMBERSHIP_RATE = 30
MEMBERSHIP_WINDOW = 600
#: The two roles a body may assign. ``owner`` never is.
ASSIGNABLE_ROLES = ("editor", "viewer")


def _uuid_or_none(value):
    """A UUID from a client-supplied member id, or None (never raises)."""
    if isinstance(value, uuid.UUID):
        return value
    try:
        return uuid.UUID(str(value))
    except (ValueError, TypeError, AttributeError):
        return None


def _member_limiter():
    return _window_limiter("playlist-members", MEMBERSHIP_RATE, MEMBERSHIP_WINDOW)


def _reset_for_tests():
    """Drop this module's membership window (tests only; see webui._window_limiter)."""
    from . import _LIMITERS

    _LIMITERS.pop("playlist-members", None)


def _too_many_members():
    return jsonify({"error": "member limit"}), 409


def _members_answer(pl, version=None):
    """The one shape every membership mutation answers."""
    return jsonify(
        {
            "members": placcess.member_rows(pl),
            "members_version": (
                pl.members_version if version is None else version
            ),
        }
    )


def _role_from_body(data):
    role = data.get("role")
    if role not in ASSIGNABLE_ROLES:
        return None, (jsonify({"error": "invalid role"}), 400)
    return role, None


@webapi.route("/playlist/<playlist_id>/version")
@login_required
def playlist_version(playlist_id):
    """The live-refresh probe: two counters and the caller's visible count.

    A collaborative page polls this every 30 s instead of the ~1 MB payload.
    A public reader may read it (they can open the page) but gets no author
    hint: the name of the last editor is not theirs to see.
    """
    pl, role, error = _playlist_guard(playlist_id, "read")
    if error:
        return error
    payload = {
        "version": pl.version,
        "members_version": pl.members_version,
        "nb_tracks": _visible_playlist_count(pl, request.webuser),
    }
    if role != "public":
        payload["last_edited_by"] = pl.last_edited_by
        payload["last_edited_at"] = (
            pl.last_edited_at.isoformat() if pl.last_edited_at else None
        )
    return jsonify(payload)


@webapi.route("/playlist/<playlist_id>/members")
@login_required
def playlist_members(playlist_id):
    """The collaborators and what the caller may do. Zero requests to open the
    sheet: it paints ``data.playlist.members`` and refreshes only when the poll
    sees ``members_version`` move."""
    pl, role, error = _playlist_guard(playlist_id, "member")
    if error:
        return error
    is_admin = bool(request.webuser.admin)
    return jsonify(
        {
            "members": placcess.member_rows(pl),
            "members_version": pl.members_version,
            "role": role,
            "can_edit": bool(is_admin or placcess.role_can_edit(role)),
            "can_manage": bool(is_admin or placcess.role_can_manage(role)),
            "admin": is_admin,
        }
    )


@webapi.route("/playlist/<playlist_id>/members", methods=["POST"])
@login_required
def playlist_invite(playlist_id):
    """Invite one EXACT account name, or change an existing member's role.

    Auto-accepted (no pending state, no notification channel), owner or admin
    only. Re-inviting an existing member updates the role and preserves their
    ``added`` date; a new member respects the 100-member cap.
    """
    pl, _role, error = _playlist_guard(playlist_id, "manage")
    if error:
        return error
    user = request.webuser
    limited = _rate_limited(_member_limiter(), f"playlist-members:{user.id}")
    if limited is not None:
        return limited
    data, error = _playlist_body(PLAYLIST_BODY_METADATA)
    if error:
        return error
    expected, error = _playlist_expected(data, "members_version")
    if error:
        return error
    role, error = _role_from_body(data)
    if error:
        return error
    username = data.get("username")
    if not isinstance(username, str) or not username.strip():
        return jsonify({"error": "invalid username"}), 400
    username = username.strip()
    if len(username) > 64:
        return jsonify({"error": "invalid username"}), 400
    target = User.get_or_none(User.name == username)
    # The comparison is exact on EVERY engine: MySQL's collation matches
    # case-insensitively, and an invite names one person, not a spelling.
    if target is None or target.name != username:
        return jsonify({"error": "unknown user"}), 404
    if target.id == user.id:
        return jsonify({"error": "self"}), 400
    if target.id == pl.user_id:
        return jsonify({"error": "owner"}), 409

    class _MemberLimit(Exception):
        pass

    try:
        with db.atomic():
            new_version = placcess.bump_members(pl, expected, user)
            row = PlaylistMember.get_or_none(
                (PlaylistMember.playlist == pl.id)
                & (PlaylistMember.user == target.id)
            )
            if row is None:
                members = (
                    PlaylistMember.select()
                    .where(PlaylistMember.playlist == pl.id)
                    .count()
                )
                if members >= PLAYLIST_MAX_MEMBERS:
                    raise _MemberLimit()
                PlaylistMember.create(
                    playlist=pl, user=target, role=role, added=now()
                )
            elif row.role != role:
                row.role = role
                row.save(only=(PlaylistMember.role,))
    except _MemberLimit:
        return _too_many_members()
    except placcess.VersionConflict as exc:
        return _playlist_conflict(exc.playlist, "members")
    return _members_answer(pl, new_version)


@webapi.route("/playlist/<playlist_id>/members/<user_id>", methods=["PATCH"])
@login_required
def playlist_member_role(playlist_id, user_id):
    """Promote/demote one member (editor <-> viewer). Owner or admin only."""
    pl, _role, error = _playlist_guard(playlist_id, "manage")
    if error:
        return error
    user = request.webuser
    limited = _rate_limited(_member_limiter(), f"playlist-members:{user.id}")
    if limited is not None:
        return limited
    target_id = _uuid_or_none(user_id)
    if target_id is None:
        return jsonify({"error": "not found"}), 404
    if target_id == pl.user_id:
        return jsonify({"error": "owner"}), 409
    data, error = _playlist_body(PLAYLIST_BODY_METADATA)
    if error:
        return error
    expected, error = _playlist_expected(data, "members_version")
    if error:
        return error
    role, error = _role_from_body(data)
    if error:
        return error
    row = PlaylistMember.get_or_none(
        (PlaylistMember.playlist == pl.id) & (PlaylistMember.user == target_id)
    )
    if row is None:
        return jsonify({"error": "not found"}), 404
    try:
        with db.atomic():
            new_version = placcess.bump_members(pl, expected, user)
            PlaylistMember.update(role=role).where(
                (PlaylistMember.playlist == pl.id)
                & (PlaylistMember.user == target_id)
            ).execute()
    except placcess.VersionConflict as exc:
        return _playlist_conflict(exc.playlist, "members")
    return _members_answer(pl, new_version)


@webapi.route("/playlist/<playlist_id>/members/<user_id>", methods=["DELETE"])
@login_required
def playlist_member_remove(playlist_id, user_id):
    """Remove one member. Idempotent: removing a non-member is a 200 with the
    current list and no version bump (a retried tap must not 404)."""
    pl, _role, error = _playlist_guard(playlist_id, "manage")
    if error:
        return error
    user = request.webuser
    limited = _rate_limited(_member_limiter(), f"playlist-members:{user.id}")
    if limited is not None:
        return limited
    target_id = _uuid_or_none(user_id)
    if target_id is None:
        return jsonify({"error": "not found"}), 404
    if target_id == pl.user_id:
        return jsonify({"error": "owner"}), 409
    data, error = _playlist_body(PLAYLIST_BODY_METADATA, allow_empty=True)
    if error:
        return error
    expected, error = _playlist_expected(data, "members_version")
    if error:
        return error
    row = PlaylistMember.get_or_none(
        (PlaylistMember.playlist == pl.id) & (PlaylistMember.user == target_id)
    )
    if row is None:
        return _members_answer(pl)
    try:
        with db.atomic():
            new_version = placcess.bump_members(pl, expected, user)
            PlaylistMember.delete().where(
                (PlaylistMember.playlist == pl.id)
                & (PlaylistMember.user == target_id)
            ).execute()
    except placcess.VersionConflict as exc:
        return _playlist_conflict(exc.playlist, "members")
    return _members_answer(pl, new_version)


@webapi.route("/playlist/<playlist_id>/leave", methods=["POST"])
@login_required
def playlist_leave(playlist_id):
    """An editor/viewer leaves. The owner cannot (409): there is no ownership
    transfer in v1, and leaving would orphan the playlist."""
    pl, role, error = _playlist_guard(playlist_id, "member")
    if error:
        return error
    user = request.webuser
    if pl.user_id == user.id:
        return jsonify({"error": "owner"}), 409
    # An admin who is NOT a member has role "admin": leave is not theirs (there
    # is no membership row to remove, and the override is not a membership).
    if role not in placcess.ROW_ROLES:
        return jsonify({"error": "not found"}), 404
    data, error = _playlist_body(PLAYLIST_BODY_METADATA, allow_empty=True)
    if error:
        return error
    try:
        with db.atomic():
            placcess.bump_members(pl, None, user)
            PlaylistMember.delete().where(
                (PlaylistMember.playlist == pl.id) & (PlaylistMember.user == user.id)
            ).execute()
    except placcess.VersionConflict as exc:  # pragma: no cover - unexpected=None
        return _playlist_conflict(exc.playlist, "members")
    return jsonify({"ok": True})
