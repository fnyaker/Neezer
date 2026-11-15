# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Who may read, edit or manage one playlist, and its two version counters.

This module is the ONE authority for playlist permissions. A route resolves the
row, asks here, and never re-derives the matrix from ``request.webuser.admin``
or from an owner id in a body (plan §5, §15.2). It imports no Flask: the same
answer is needed from a request, from Subsonic and from a worker.

The rules, in order:

* the owner is ``Playlist.user_id`` FIRST — a playlist created by an older path
  (or by a test, or by the account importer) may have no owner row in
  ``playlist_member``, and that must never lock its owner out. The row is
  displayed anyway, synthesized by :func:`member_rows`;
* then the stored member role, and only ``editor``/``viewer`` are ever accepted
  from the table: authorization never trusts a value the routes cannot write;
* then the instance admin override — a non-member admin gets ``"admin"``, an
  admin who IS a member keeps that member role plus the override (``can_edit``
  and ``can_manage`` are true either way);
* then ``Playlist.public``: readable by any logged-in account by URL, never
  writable, never listed in anybody's ``/api/me/playlists``.

A non-member gets ``None`` — every web route turns that into **404**, not 403,
so a real-but-foreign playlist is indistinguishable from an unknown id.

The two counters (schema ``20261114``) guard two different things, so an invite
never invalidates a reorder in flight:

* ``version`` — the tracklist and the title/description. Bumped by every
  order-sensitive mutation, by the background writers (importer, Subsonic) and
  by a version-less legacy client;
* ``members_version`` — who is on the playlist (invite, role, removal, leave).

:func:`bump_version` / :func:`bump_members` are **compare-and-swap** statements:
the ``UPDATE`` is the serialization point (a row lock on PostgreSQL/MySQL, the
write lock on SQLite), so two clients that send the same expected version get
exactly one winner and one :class:`VersionConflict` (409 + the authoritative
counters, never a merged order). ``expected=None`` means "the client cannot
send a version" (an older installed SPA, a background writer): unconditional,
then the new value is read back.
"""

from __future__ import annotations

import datetime

from .db import Playlist, PlaylistMember, User, now

#: The only roles ever handed out. ``owner`` is computed, never assigned.
ROLES = ("owner", "editor", "viewer")
#: The roles a member row may hold; anything else is treated as no membership.
ROW_ROLES = ("editor", "viewer")
#: A role that may change the tracklist.
EDIT_ROLES = ("owner", "editor", "admin")
#: A role that may rename, delete, invite or manage the public link.
MANAGE_ROLES = ("owner", "admin")


class VersionConflict(Exception):
    """The playlist moved under the client; the route answers 409 + counters."""

    def __init__(self, playlist):
        super().__init__("playlist version conflict")
        self.playlist = playlist


def role_can_edit(role) -> bool:
    """The tracklist right, from a role already resolved by :func:`role_for`."""
    return role in EDIT_ROLES


def role_can_manage(role) -> bool:
    """The owner rights (rename, delete, invite, public link)."""
    return role in MANAGE_ROLES


def member_role(playlist, user):
    """The STORED role of ``user`` on ``playlist``, or None. One query.

    The owner answers without touching ``playlist_member``: ``Playlist.user_id``
    is the primary source, so a missing owner row (an older creation path) can
    never demote or lock out the owner. Only ``editor``/``viewer`` are accepted
    from the table.
    """
    if playlist is None or user is None:
        return None
    if playlist.user_id == user.id:
        return "owner"
    role = (
        PlaylistMember.select(PlaylistMember.role)
        .where(
            (PlaylistMember.playlist == playlist.id)
            & (PlaylistMember.user == user.id)
        )
        .scalar()
    )
    return role if role in ROW_ROLES else None


def role_for(playlist, user, member=None):
    """``owner | editor | viewer | admin | public | None`` for one playlist.

    `member` is :func:`member_role`'s answer when the caller already has it, so
    a route that fetched the role once does not pay a second query per
    predicate. The order is the authority: owner from ``user_id``, then the
    member row, then the admin override, then ``Playlist.public``.
    """
    if playlist is None or user is None:
        return None
    if playlist.user_id == user.id:
        return "owner"
    if member is None:
        member = member_role(playlist, user)
    if member in ROW_ROLES:
        return member
    if getattr(user, "admin", False):
        return "admin"
    if playlist.public:
        return "public"
    return None


def can_read(playlist, user) -> bool:
    """May ``user`` see the playlist, its visible tracks and its suggestions?"""
    return role_for(playlist, user) is not None


def can_edit(playlist, user) -> bool:
    """May ``user`` add, remove and reorder tracks? Owner, editor, admin."""
    return role_can_edit(role_for(playlist, user))


def can_manage(playlist, user) -> bool:
    """May ``user`` rename, delete, invite, change roles, manage the link?"""
    return role_can_manage(role_for(playlist, user))


def _added_key(added):
    """A comparable key for ``added``; a missing date sorts first."""
    return added or datetime.datetime.min


def member_rows(playlist):
    """The playlist's people, owner first: ``[{id, name, role, added}]``.

    One query for the non-owner rows, joined to the account for its name. The
    owner is synthesized from ``Playlist.user`` when their row is missing (a
    playlist created before the migration, by the importer, or directly by a
    test), so the page always shows its owner. The owner is pinned first; then
    editors, then viewers, each by ``added`` then name.

    A stored role outside ``editor``/``viewer`` is displayed as ``viewer``: the
    row stays visible, but a corrupt value never reads as a right. ``added`` is
    an ISO string because that is the shape the JSON contract carries.
    """
    owner_id = playlist.user_id
    owner_added = None
    owner_name = None
    others = []
    rows = (
        PlaylistMember.select(
            PlaylistMember.user,
            PlaylistMember.role,
            PlaylistMember.added,
            User.name,
        )
        .join(User, on=(PlaylistMember.user == User.id))
        .where(PlaylistMember.playlist == playlist.id)
        .tuples()
    )
    for uid, role, added, name in rows:
        if uid == owner_id:
            owner_added, owner_name = added, name
            continue
        if role not in ROW_ROLES:
            role = "viewer"
        others.append({"id": str(uid), "name": name, "role": role, "added": added})
    # The owner's account is read through the FK (one query, cached on the
    # Playlist instance) ONLY when their member row is missing — a playlist
    # created before the migration, by the importer, or directly by a test. The
    # members list must never be ownerless.
    if owner_name is None:
        owner_name = playlist.user.name
    owner_when = owner_added or playlist.created or now()
    others.sort(
        key=lambda row: (
            0 if row["role"] == "editor" else 1,
            _added_key(row["added"]),
            row["name"] or "",
        )
    )
    out = [
        {
            "id": str(owner_id),
            "name": owner_name,
            "role": "owner",
            "added": owner_when.isoformat(),
        }
    ]
    for row in others:
        out.append(
            {
                "id": row["id"],
                "name": row["name"],
                "role": row["role"],
                "added": row["added"].isoformat() if row["added"] else None,
            }
        )
    return out


def bump_version(playlist, expected, actor):
    """CAS ``Playlist.version`` and stamp the editor; return the new version.

    Must run inside the caller's ``db.atomic()``, together with the track write
    it guards: the ``UPDATE`` takes the row lock, and a crash rolls the counter
    back with the rows. ``expected`` is the version the client read; a mismatch
    raises :class:`VersionConflict`. ``expected=None`` is the legacy/background
    path: unconditional, then the committed value is read back.
    """
    new_at = now()
    name = getattr(actor, "name", None)
    query = Playlist.update(
        version=Playlist.version + 1,
        last_edited_by=name,
        last_edited_at=new_at,
    ).where(Playlist.id == playlist.id)
    if expected is not None:
        query = query.where(Playlist.version == expected)
    if query.execute() == 0:
        raise VersionConflict(playlist)
    if expected is None:
        playlist.version = (
            Playlist.select(Playlist.version)
            .where(Playlist.id == playlist.id)
            .scalar()
        )
    else:
        playlist.version = expected + 1
    playlist.last_edited_by = name
    playlist.last_edited_at = new_at
    return playlist.version


def bump_members(playlist, expected, actor):
    """CAS ``Playlist.members_version``; return the new value.

    A membership mutation carries ONLY this counter: an invite must not make a
    collaborator's in-flight reorder conflict, and the last content author
    (``last_edited_by``) is deliberately not stamped by a role change.
    """
    query = Playlist.update(
        members_version=Playlist.members_version + 1,
    ).where(Playlist.id == playlist.id)
    if expected is not None:
        query = query.where(Playlist.members_version == expected)
    if query.execute() == 0:
        raise VersionConflict(playlist)
    if expected is None:
        playlist.members_version = (
            Playlist.select(Playlist.members_version)
            .where(Playlist.id == playlist.id)
            .scalar()
        )
    else:
        playlist.members_version = expected + 1
    return playlist.members_version
