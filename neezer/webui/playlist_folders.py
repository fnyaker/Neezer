# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Playlist folders: a private, per-person organisation of playlists.

A folder groups PLAYLISTS, never tracks, belongs to ONE person, and is never
written anywhere but this database (no provider is asked, nothing on a Deezer
account changes). Two tables (``neezer/db.py``, schema ``20261114``):

* ``playlist_folder`` — ``name``, ``position`` (the order of the person's own
  folders), one level, no parent;
* ``playlist_placement`` — ``(user, playlist)`` → ``folder`` (NULL = the
  unfiled root) and ``position``. Placement is a *view*: two people file the
  same imported playlist independently and neither sees the other's folders.
  A playlist with NO row is "never ordered" and sorts after the ordered ones by
  creation date, which is exactly the list every existing server already had —
  so the upgrade needs no backfill.

The five mutations live here, plus the ordering rule shared with
``GET /api/me/playlists`` (``list_shape``) so the shape the SPA renders is
decided in one file:

* folder CRUD — create is idempotent through the client-generated id (a
  retried POST after a timeout returns the same folder, not a second one);
* ``PUT /api/playlist-folders/order`` — the complete desired order of the
  person's folders;
* ``PUT /api/playlist-folders/<id>/order`` and ``/root/order`` — the complete
  desired order of ONE container. The listed playlists are filed/moved into it
  in that order, then any current resident the client omitted is APPENDED in
  its existing order, so a partial list is never destructive. Positions are
  renormalised to ``0..n-1`` by one DELETE plus chunked INSERTs inside one
  transaction (never an UPDATE per row).

What this deliberately does NOT do: delete a playlist when its folder goes
(they return to « Sans dossier » and the response carries their previous order,
which is what undo re-applies), touch ``/rest`` (a folder is not a Subsonic
concept and no accepted OpenSubsonic extension exists), or nest folders in v1.

Concurrency is **last write wins**: there is no ETag/version on an order. A
lost reorder never loses a playlist or a folder — the only destructive
operation is the folder delete, which is undoable and never touches a
``Playlist`` row.

Every route is ``@login_required`` and the blueprint installs its OWN
same-origin guard (``webui/__init__.py``'s is per blueprint and a new one does
not inherit it); ``remote.enforce_remote_scope`` is application-level. A
foreign id answers 404 (or, where the route is idempotent, an empty success) so
folder ids cannot be probed.
"""

from __future__ import annotations

import datetime
import logging
import unicodedata
import uuid

from flask import Blueprint, jsonify, request
from peewee import Case, IntegrityError, fn

from ..db import Playlist, PlaylistFolder, PlaylistPlacement, db

from . import _uuid_text, login_required, same_origin_guard

logger = logging.getLogger(__name__)

playlist_folders_api = Blueprint("playlist_folders_api", __name__)

#: A folder name is short on purpose: it is a 232 px sidebar row and a 44 px
#: phone heading, and the sheet's input carries ``maxlength=80``.
MAX_TITLE = 80
#: One person's folder list. Bounded so a hostile body cannot make the server
#: sort and write an unbounded list; a real library is far below it.
MAX_FOLDERS = 500
#: One container's desired order (a phone can hold 500 playlists; 2 000 is the
#: ceiling that still fits a single request and a few chunked statements).
MAX_PLAYLISTS = 2000
#: IN lists are chunked like the rest of ``webui`` (SQLite's default cap is 999
#: bound parameters on older builds); inserts follow ``reorder_playlist``'s
#: pattern but at 240 rows, because a placement carries FOUR columns per row
#: (960 bound parameters) where a playlist track carries three.
_IN_CHUNK = 400
_INSERT_CHUNK = 240
#: A folder's position is a COLUMN, so renormalising it is one ``CASE`` update
#: per chunk instead of an UPDATE per row (and never a delete + insert: that
#: would cascade the placements away).
_UPDATE_CHUNK = 100


# -- the same same-origin guard as the rest of /api ---------------------------
# ``webui/__init__.py`` installs it on ``webapi``; a blueprint does not inherit
# another blueprint's before_request hooks, so this must be here explicitly.
@playlist_folders_api.before_request
def reject_cross_site():
    return same_origin_guard()


@playlist_folders_api.errorhandler(Exception)
def _json_error(exc):
    from werkzeug.exceptions import HTTPException

    if isinstance(exc, HTTPException):
        return jsonify({"error": exc.name}), exc.code
    logger.error("Unhandled error on %s %s", request.method, request.path, exc_info=True)
    return jsonify({"error": "server error"}), 500


# -- validation ---------------------------------------------------------------


def _json_body():
    """``(dict, None)`` for a JSON object body, ``(None, response)`` otherwise.

    An empty body, a non-JSON body and a JSON array all answer 400 — never a
    500 (the /api coverage walk POSTs every route with no body at all).
    """
    data = request.get_json(silent=True)
    if not isinstance(data, dict):
        return None, (jsonify({"error": "invalid body"}), 400)
    return data, None


def _uuid_id(value):
    """A ``uuid.UUID`` from a client-supplied id, or None.

    Never raises: a list, a number, a very long string and an empty string are
    all "not an id".
    """
    if isinstance(value, uuid.UUID):
        return value
    if not isinstance(value, str):
        return None
    try:
        return uuid.UUID(value)
    except (ValueError, TypeError, AttributeError):
        return None


def _title(data):
    """``(title, None)`` or ``(None, response)``: 1..80 chars, no control one.

    The control-character refusal is not cosmetic: a newline in a folder name
    reflows the sidebar row, and a NUL is refused by MySQL outright (a 500 on
    one engine and a 201 on another is the kind of split a test would miss).
    """
    raw = data.get("title")
    if not isinstance(raw, str) or not raw.strip():
        return None, (jsonify({"error": "missing title"}), 400)
    title = raw.strip()
    if len(title) > MAX_TITLE:
        return None, (jsonify({"error": "title too long"}), 400)
    if any(unicodedata.category(c) == "Cc" for c in title):
        return None, (jsonify({"error": "invalid title"}), 400)
    return title, None


def _container_clause(folder):
    """The placements of one container: ``folder`` None is the unfiled root."""
    if folder is None:
        return PlaylistPlacement.folder.is_null(True)
    return PlaylistPlacement.folder == folder


def resolve_folder(user, value):
    """``(folder or None, error_response)`` for a client-supplied folder id.

    ``None`` is the unfiled root (a valid target). Anything else must parse as
    a UUID and belong to ``user``: a foreign folder is a 404, never a 403, so
    the answer never confirms that an id exists.
    """
    if value is None:
        return None, None
    fid = _uuid_id(value)
    if fid is None:
        return None, (jsonify({"error": "invalid id"}), 400)
    folder = PlaylistFolder.get_or_none(
        (PlaylistFolder.id == fid) & (PlaylistFolder.user == user)
    )
    if folder is None:
        return None, (jsonify({"error": "folder not found"}), 404)
    return folder, None


# -- the ordering rule --------------------------------------------------------


def _folder_rows(user):
    """``[(id, name, position, created)]`` in the order the SPA renders them."""
    rows = list(
        PlaylistFolder.select(
            PlaylistFolder.id, PlaylistFolder.name, PlaylistFolder.position, PlaylistFolder.created
        )
        .where(PlaylistFolder.user == user)
        .tuples()
    )
    # Sorted here, not in SQL: `id`'s native order differs between a uuid column
    # (PostgreSQL) and its text form (SQLite/MySQL), and the tie-break must be
    # the same everywhere. ≤ 500 rows, one pass.
    rows.sort(key=lambda r: (r[2], _creation_key(r[3]), str(r[0])))
    return rows


def _creation_key(created, descending=False):
    """``created`` as a comparable pair, the way the container orders it.

    Built from the date's own fields rather than ``timestamp()``: a naive
    datetime is ambiguous around a DST fall-back, and the order must not depend
    on the server's timezone. Microseconds are kept, so a hand-set timestamp
    (the tests set minutes apart, seconds are the normal precision) still sorts
    exactly.
    """
    created = created or datetime.datetime.min
    stamp = (
        created.toordinal() * 86400
        + created.hour * 3600
        + created.minute * 60
        + created.second,
        created.microsecond,
    )
    return (-stamp[0], -stamp[1]) if descending else stamp


def _entry_key(entry):
    """§4.3, as one key: container, then placement, then creation date.

    A row WITH a placement sorts before one without (``position is None``), by
    ``position`` ASC, then ``created`` ASC, then ``id``; a row WITHOUT one
    sorts last, by ``created`` DESC, then ``id`` — today's list.
    """
    placed = entry["position"] is not None
    return (
        entry["rank"],
        not placed,
        entry["position"] if placed else 0,
        _creation_key(entry["created"], descending=not placed),
        entry["key"],
    )


def list_shape(user, rows):
    """Order one person's playlists the way ``/api/me/playlists`` renders them.

    ``rows`` are ``(raw id, deezer_id, name, created, raw folder id, position)``
    read by the route in ONE left join on the caller's placements. Returns
    ``(folders, entries)``:

    * ``folders`` — ``[{"id", "title", "position"}]`` in folder order
      (``position``, then ``created``, then ``id``; ties cannot be ambiguous, so
      a concurrent create never makes the list flicker between two requests);
    * ``entries`` — one dict per playlist in render order, with ``pid`` (the
      raw id, so the caller attaches its counts and covers without a second
      lookup), ``folder_id``, ``position`` and the sort keys
      (``rank``, ``created``, ``key``): each folder's playlists contiguous
      (folders in folder order), then the unfiled root. Within one container
      the rows WITH a placement come first (``position`` ASC, then ``created``,
      then ``id``), then the ones without (``created`` DESC, then ``id``) —
      today's order, byte for byte.

    A placement pointing at a folder this person does not own any more (a row
    the DB cascade should already have removed) is shown as unfiled rather than
    as a member of a folder the client cannot name.
    """
    folder_rows = _folder_rows(user)
    folders = [
        {"id": str(fid), "title": name, "position": position}
        for fid, name, position, _created in folder_rows
    ]
    rank = {str(fid): i for i, (fid, _n, _p, _c) in enumerate(folder_rows)}
    root_rank = len(folder_rows)

    entries = []
    for pid, _dz, _name, created, folder_raw, position in rows:
        if folder_raw is None:
            # No placement row, or a placement in the unfiled root: both are
            # the root container, and the position (when there is one) is real.
            fid, place = None, root_rank
        else:
            fid = _uuid_text(folder_raw)
            place = rank.get(fid)
            if place is None:
                # A placement pointing at a folder this person does not own any
                # more: shown as unfiled, never as a member of a folder the
                # client cannot name.
                fid, position, place = None, None, root_rank
        entries.append(
            {
                "pid": pid,
                "folder_id": fid,
                "position": position,
                "rank": place,
                "created": created,
                "key": _uuid_text(pid),
            }
        )
    # One key (``_entry_key``), not two stable sorts: the rows WITH a placement
    # order by an ASCENDING creation date and the unfiled tail by a DESCENDING
    # one, and a single direction cannot compose in two passes.
    entries.sort(key=_entry_key)
    return folders, entries


# -- placement writes ---------------------------------------------------------


def place_new_playlist(user, playlist, folder=None):
    """Put a freshly created playlist at the TOP of its container.

    ``position = COALESCE(MIN(position), 0) - 1`` of the destination, so a
    create never rewrites the container: negative positions are allowed and
    sort first, which is the visible behaviour the SPA already assumes (it
    prepends the new playlist locally). Two statements, one transaction.
    Returns the position written.
    """
    with db.atomic():
        lowest = (
            PlaylistPlacement.select(fn.MIN(PlaylistPlacement.position))
            .where((PlaylistPlacement.user == user) & _container_clause(folder))
            .scalar()
        )
        position = -1 if lowest is None else lowest - 1
        PlaylistPlacement.create(
            user=user, playlist=playlist, folder=folder, position=position
        )
    return position


def _residents(user, fid):
    """``(folder exists, rows)`` for one container — ONE statement.

    ``rows`` are ``[(playlist id, created, position)]``: the playlists this
    person may see that are currently in the container (the folder when ``fid``
    is given, the unfiled root when it is None), in container order (position,
    then creation date, then id). The visibility filter is the same one
    ``/api/me/playlists`` uses: the caller's own lists, or a Deezer import. A
    placement left pointing at a playlist they can no longer see is inert and
    is not carried over (it is dropped by the rewrite).

    For a folder the ownership check rides in the same SELECT (a LEFT JOIN off
    ``playlist_folder``): an existing folder with no playlist still yields one
    all-NULL row, so "empty" and "not yours" are told apart without a second
    round trip.
    """
    visible = (Playlist.user == user) | (Playlist.deezer_id.is_null(False))
    if fid is None:
        rows = list(
            PlaylistPlacement.select(Playlist.id, Playlist.created, PlaylistPlacement.position)
            .join(Playlist, on=(PlaylistPlacement.playlist == Playlist.id))
            .where(
                (PlaylistPlacement.user == user)
                & PlaylistPlacement.folder.is_null(True)
                & visible
            )
            .tuples()
        )
        exists = True
    else:
        raw = list(
            PlaylistFolder.select(
                PlaylistFolder.id, Playlist.id, Playlist.created, PlaylistPlacement.position
            )
            .left_outer_join(
                PlaylistPlacement,
                on=((PlaylistPlacement.folder == PlaylistFolder.id) & (PlaylistPlacement.user == user)),
            )
            .switch(PlaylistFolder)
            .left_outer_join(
                Playlist,
                on=((PlaylistPlacement.playlist == Playlist.id) & visible),
            )
            .where((PlaylistFolder.id == fid) & (PlaylistFolder.user == user))
            .tuples()
        )
        exists = bool(raw)
        rows = [(pid, created, position) for _f, pid, created, position in raw if pid is not None]
    rows.sort(key=lambda r: (r[2], _creation_key(r[1]), str(r[0])))
    return exists, rows


def _visible(user, ids):
    """The subset of ``ids`` this person may file: own, or a Deezer import.

    Exactly what ``/api/me/playlists`` lists. One statement per 400 ids; an id
    the caller cannot see is simply left out of the order (never a 403, and
    never a row).
    """
    out = set()
    for i in range(0, len(ids), _IN_CHUNK):
        chunk = ids[i : i + _IN_CHUNK]
        out.update(
            pid
            for pid, in (
                Playlist.select(Playlist.id)
                .where(
                    Playlist.id.in_(chunk)
                    & ((Playlist.user == user) | (Playlist.deezer_id.is_null(False)))
                )
                .tuples()
            )
        )
    return out


# -- folder CRUD --------------------------------------------------------------


@playlist_folders_api.route("/playlist-folders", methods=["POST"])
@login_required
def create_playlist_folder():
    """Create one folder. ``{"id"?: uuid, "title": "1..80"}`` → 201.

    The id is CLIENT-generated (``crypto.randomUUID()`` in the SPA) so a POST
    retried after a timeout returns the very same folder with a 200 instead of
    creating a second one. Position is appended (``MAX + 1``), which needs no
    rewrite of the existing order.
    """
    data, error = _json_body()
    if error:
        return error
    title, error = _title(data)
    if error:
        return error
    raw_id = data.get("id")
    fid = None
    if raw_id is not None:
        fid = _uuid_id(raw_id)
        if fid is None:
            return jsonify({"error": "invalid id"}), 400
    user = request.webuser
    if fid is not None:
        existing = PlaylistFolder.get_or_none(PlaylistFolder.id == fid)
        if existing is not None:
            if existing.user_id != user.id:
                # Never confirm that somebody else's id exists: the same 400 a
                # malformed id gets.
                return jsonify({"error": "invalid id"}), 400
            return jsonify(
                {
                    "ok": True,
                    "id": str(existing.id),
                    "title": existing.name,
                    "position": existing.position,
                }
            )
    new_id = fid or uuid.uuid4()
    highest = (
        PlaylistFolder.select(fn.MAX(PlaylistFolder.position))
        .where(PlaylistFolder.user == user)
        .scalar()
    )
    try:
        # Its own ``atomic()``: a real transaction standalone, a SAVEPOINT when
        # the caller is already in one, so a clash rolls back just this insert
        # (``deezer.library.create_or_get`` states the rule for every upsert).
        with db.atomic():
            folder = PlaylistFolder.create(
                id=new_id,
                user=user,
                name=title,
                position=0 if highest is None else highest + 1,
            )
    except IntegrityError:
        # Two taps of one client id (or an id somebody else already owns, which
        # must not be confirmed to exist): read the winner back, or answer the
        # same 400 a malformed id gets.
        folder = PlaylistFolder.get_or_none(
            (PlaylistFolder.id == new_id) & (PlaylistFolder.user == user)
        )
        if folder is None:
            return jsonify({"error": "invalid id"}), 400
        return jsonify(
            {
                "ok": True,
                "id": str(folder.id),
                "title": folder.name,
                "position": folder.position,
            }
        )
    return (
        jsonify(
            {
                "ok": True,
                "id": str(folder.id),
                "title": folder.name,
                "position": folder.position,
            }
        ),
        201,
    )


@playlist_folders_api.route("/playlist-folders/<folder_id>", methods=["PATCH"])
@login_required
def edit_playlist_folder(folder_id):
    """Rename one of the caller's folders. ``{"title": "1..80"}`` → 200."""
    fid = _uuid_id(folder_id)
    if fid is None:
        return jsonify({"error": "invalid id"}), 400
    data, error = _json_body()
    if error:
        return error
    title, error = _title(data)
    if error:
        return error
    folder = PlaylistFolder.get_or_none(
        (PlaylistFolder.id == fid) & (PlaylistFolder.user == request.webuser)
    )
    if folder is None:
        return jsonify({"error": "folder not found"}), 404
    folder.name = title
    folder.save(only=(PlaylistFolder.name,))
    return jsonify({"ok": True, "id": str(folder.id), "title": folder.name})


@playlist_folders_api.route("/playlist-folders/<folder_id>", methods=["DELETE"])
@login_required
def delete_playlist_folder(folder_id):
    """Delete one folder and nothing else.

    Its placements go with it (``ON DELETE CASCADE``), so its playlists become
    unfiled and sort at the tail of « Sans dossier » by creation date; no
    ``Playlist`` row is touched. The response carries their previous order,
    which is what the SPA's undo re-applies (re-create the folder with the same
    id, then one ``PUT <id>/order``).

    An unknown or foreign id is an idempotent ``{"ok": true, "playlists": []}``
    — never a 404, so the answer is no oracle for whether a folder exists.
    """
    fid = _uuid_id(folder_id)
    if fid is None:
        return jsonify({"error": "invalid id"}), 400
    user = request.webuser
    rows = list(
        PlaylistPlacement.select(Playlist.id, Playlist.created, PlaylistPlacement.position)
        .join(Playlist, on=(PlaylistPlacement.playlist == Playlist.id))
        .where((PlaylistPlacement.user == user) & (PlaylistPlacement.folder == fid))
        .tuples()
    )
    rows.sort(key=lambda r: (r[2], r[1], str(r[0])))
    PlaylistFolder.delete().where(
        (PlaylistFolder.id == fid) & (PlaylistFolder.user == user)
    ).execute()
    return jsonify({"ok": True, "playlists": [str(pid) for pid, _c, _p in rows]})


# -- folder order -------------------------------------------------------------


@playlist_folders_api.route("/playlist-folders/order", methods=["PUT"])
@login_required
def reorder_playlist_folders():
    """The complete desired order of the caller's folders.

    ``{"folders": [id, …]}``. Unknown ids are ignored, duplicates collapse
    (first occurrence wins), and any folder the client omitted is appended in
    its existing order — so a stale or partial list is never destructive.
    Positions are renormalised to ``0..n-1``; the same PUT twice yields the same
    rows.
    """
    data, error = _json_body()
    if error:
        return error
    raw = data.get("folders")
    if raw is None or (isinstance(raw, list) and not raw):
        return jsonify({"error": "no order"}), 400
    if not isinstance(raw, list):
        return jsonify({"error": "invalid order"}), 400
    if len(raw) > MAX_FOLDERS:
        return jsonify({"error": "too many folders"}), 400
    wanted, seen = [], set()
    for value in raw:
        if not isinstance(value, str):
            return jsonify({"error": "invalid order"}), 400
        fid = _uuid_id(value)
        if fid is None:
            return jsonify({"error": "invalid order"}), 400
        if fid in seen:
            continue
        seen.add(fid)
        wanted.append(fid)

    user = request.webuser
    current = _folder_rows(user)
    by_id = {fid: position for fid, _name, position, _created in current}
    order = [fid for fid in wanted if fid in by_id]
    listed = set(order)
    order += [fid for fid, _n, _p, _c in current if fid not in listed]

    with db.atomic():
        for start in range(0, len(order), _UPDATE_CHUNK):
            chunk = order[start : start + _UPDATE_CHUNK]
            # One statement per chunk: a position is a column, so a row-by-row
            # UPDATE loop would be 500 round-trips on a mass reorder.
            position = Case(None, [(PlaylistFolder.id == fid, start + i) for i, fid in enumerate(chunk)])
            PlaylistFolder.update(position=position).where(
                (PlaylistFolder.user == user) & PlaylistFolder.id.in_(chunk)
            ).execute()
    return jsonify({"ok": True, "folders": [str(fid) for fid in order]})


# -- container order ----------------------------------------------------------


@playlist_folders_api.route("/playlist-folders/<folder_id>/order", methods=["PUT"])
@playlist_folders_api.route("/playlist-folders/root/order", methods=["PUT"])
@login_required
def reorder_container(folder_id=None):
    """The complete desired order of ONE container (a folder, or the root).

    ``{"playlists": [id, …]}``. The listed playlists are filed/moved into the
    container in that order; a listed id this person cannot see is ignored
    (exactly ``/api/me/playlists``'s rule: their own, or a Deezer import), and
    any current resident the client omitted is appended in its existing order.
    Moving a playlist into a folder is therefore the same call as reordering
    the destination — the SPA updates both containers locally and sends one
    request.

    One transaction: the destination's placements are deleted, the moved ones
    are removed wherever they were (the composite key is
    ``(user, playlist)``), then the whole order is inserted in chunks with
    positions ``0..n-1``. Idempotent; last write wins.
    """
    fid = None
    if folder_id not in (None, "root"):
        fid = _uuid_id(folder_id)
        if fid is None:
            return jsonify({"error": "invalid id"}), 400
    data, error = _json_body()
    if error:
        return error
    raw = data.get("playlists")
    if not isinstance(raw, list) or not raw:
        return jsonify({"error": "no playlists"}), 400
    if len(raw) > MAX_PLAYLISTS:
        return jsonify({"error": "too many playlists"}), 400
    wanted, seen = [], set()
    for value in raw:
        pid = _uuid_id(value)
        if pid is None or pid in seen:
            continue
        seen.add(pid)
        wanted.append(pid)

    user = request.webuser
    exists, residents = _residents(user, fid)
    if not exists:
        return jsonify({"error": "folder not found"}), 404
    resident_ids = {pid for pid, _c, _p in residents}
    # A listed id that is already in this container was checked by the read
    # above; only the ones coming from elsewhere need the visibility query (and
    # for a plain reorder there are none of those, so it costs no statement).
    elsewhere = [pid for pid in wanted if pid not in resident_ids]
    visible_elsewhere = _visible(user, elsewhere) if elsewhere else set()
    kept = [pid for pid in wanted if pid in resident_ids or pid in visible_elsewhere]
    order = list(kept)
    listed = set(order)
    for pid, _created, _position in residents:
        if pid not in listed:
            order.append(pid)
            listed.add(pid)
    # A playlist that was in ANOTHER container still has its placement row (the
    # composite key is (user, playlist)): the write below removes it wherever it
    # was, in the same transaction.
    moved = [pid for pid in kept if pid not in resident_ids]

    rows = [(user.id, pid, fid, i) for i, pid in enumerate(order)]
    fields = (
        PlaylistPlacement.user,
        PlaylistPlacement.playlist,
        PlaylistPlacement.folder,
        PlaylistPlacement.position,
    )
    with db.atomic():
        if moved:
            for start in range(0, len(moved), _IN_CHUNK):
                PlaylistPlacement.delete().where(
                    (PlaylistPlacement.user == user)
                    & (
                        _container_clause(fid)
                        | PlaylistPlacement.playlist.in_(moved[start : start + _IN_CHUNK])
                    )
                ).execute()
        else:
            PlaylistPlacement.delete().where(
                (PlaylistPlacement.user == user) & _container_clause(fid)
            ).execute()
        for start in range(0, len(rows), _INSERT_CHUNK):
            PlaylistPlacement.insert_many(rows[start : start + _INSERT_CHUNK], fields=fields).execute()
    return jsonify(
        {
            "ok": True,
            "folder": str(fid) if fid is not None else None,
            "playlists": [str(pid) for pid in order],
        }
    )
