# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""A person's own listening history, read back.

The rows already exist in ``listen_event`` (``neezer/listening.py`` records
every track change with how long it played and whether it was a listen or a
skip). This module only READS them, for the History screen.

Contract:

* ``GET /api/history?before=&before_id=&limit=&outcome=&days=``
  -> ``{events, next, summary}``

Only the signed-in person's rows are served; an admin has no view of somebody
else's history. Newest first, ``at DESC, id DESC``, with a keyset cursor so a
page boundary can neither drop nor repeat an event (an ``OFFSET`` would, as
soon as a play is recorded while somebody is paging). The tracks come back in
the standard list payload (``_db_tracks``), one statement for the whole page.

The summary is the same read the screens already show: the last 30 days by
default, five artists and five titles by number of LISTENS (a skip is not a
listen), plus the number of listens, skips and listened minutes in the window.
The minute count is the time actually spent: every event's ``listened``
seconds, skips included, floored to the minute. That is what a person means by
"minutes écoutées" and it makes the three figures add up to the log below.

No statement is issued per row and nothing here calls a provider.
"""

from __future__ import annotations

import datetime

import peewee
from flask import jsonify, request
from peewee import fn

from ..db import Album, Artist, ListenEvent, Track, now
from . import _db_tracks, _uuid_text, _visible_tracks, login_required, webapi

DEFAULT_LIMIT = 50
MAX_LIMIT = 200
DEFAULT_DAYS = 30
MAX_DAYS = 365
TOP = 5


def _bad(message: str):
    return jsonify({"error": message}), 400


def _limit_arg() -> int:
    raw = request.args.get("limit")
    if raw is None or raw == "":
        return DEFAULT_LIMIT
    try:
        value = int(raw)
    except (TypeError, ValueError):
        raise ValueError("invalid limit")
    # A hostile or buggy client gets the documented bound, not a 500 and not
    # an unbounded scan.
    return max(1, min(MAX_LIMIT, value))


def _days_arg() -> int:
    raw = request.args.get("days")
    if raw is None or raw == "":
        return DEFAULT_DAYS
    try:
        value = int(raw)
    except (TypeError, ValueError):
        raise ValueError("invalid days")
    return max(1, min(MAX_DAYS, value))


def _cursor():
    """``(before, before_id)``, or ``(None, None)`` for the first page.

    Both halves are required together: a cursor with a time but no id cannot
    order the events a bigger page boundary put on the same second, and
    treating it as "first page" would silently loop over the newest rows.
    """
    raw_before = request.args.get("before")
    raw_id = request.args.get("before_id")
    if raw_before in (None, "") and raw_id in (None, ""):
        return None, None
    if raw_before in (None, "") or raw_id in (None, ""):
        raise ValueError("invalid cursor")
    text = raw_before.strip().replace("Z", "+00:00")
    try:
        before = datetime.datetime.fromisoformat(text)
    except ValueError:
        raise ValueError("invalid cursor")
    if before.tzinfo is not None:
        # Everything stored is the server's local naive time (``db.now``).
        before = before.astimezone().replace(tzinfo=None)
    try:
        before_id = int(raw_id)
    except (TypeError, ValueError):
        raise ValueError("invalid cursor")
    return before, before_id


def _outcome_arg():
    raw = request.args.get("outcome")
    if raw in (None, ""):
        return None
    if raw not in ("listen", "skip"):
        raise ValueError("invalid outcome")
    return raw


def _track_payloads(ids) -> dict:
    """The standard API dicts for ``ids``, keyed by the track UUID as text.

    One statement for the tracks (itself read as tuples) and the credits'
    own statement: the same batched path every list uses, never one per row.
    """
    ids = [i for i in ids if i is not None]
    if not ids:
        return {}
    rows = _db_tracks(
        _visible_tracks(
            Track.select(Track, Album, Artist)
            .join(Album)
            .switch(Track)
            .join(Artist)
            .where(Track.id.in_(ids))
        )
    )
    return {row["id"]: row for row in rows}


def _events_query(user, before, before_id, outcome):
    query = ListenEvent.select(
        ListenEvent.id,
        ListenEvent.at,
        ListenEvent.listened,
        ListenEvent.outcome,
        ListenEvent.source,
        ListenEvent.track,
    ).where(ListenEvent.user == user)
    if outcome == "listen":
        query = query.where(ListenEvent.outcome > 0)
    elif outcome == "skip":
        query = query.where(ListenEvent.outcome < 0)
    if before is not None:
        # Strictly older than the cursor pair: the same second is ordered by
        # id, so an event recorded between two pages can never be skipped or
        # served twice.
        query = query.where(
            (ListenEvent.at < before)
            | ((ListenEvent.at == before) & (ListenEvent.id < before_id))
        )
    return query.order_by(ListenEvent.at.desc(), ListenEvent.id.desc())


def _event_payload(row, tracks) -> dict | None:
    eid, at, listened, outcome, source, tid = row
    track = tracks.get(str(tid))
    if track is None:
        return None
    return {
        "id": eid,
        "at": at.isoformat(),
        "listened": int(listened or 0),
        "outcome": int(outcome or 0),
        "source": source,
        "track": track,
    }


def _summary(user, days: int) -> dict:
    since = now() - datetime.timedelta(days=days)
    window = (ListenEvent.user == user) & (ListenEvent.at >= since)

    _total, listens, skips, seconds = (
        ListenEvent.select(
            fn.COUNT(ListenEvent.id),
            fn.SUM(peewee.Case(None, [(ListenEvent.outcome > 0, 1)], 0)),
            fn.SUM(peewee.Case(None, [(ListenEvent.outcome < 0, 1)], 0)),
            fn.SUM(ListenEvent.listened),
        )
        .where(window)
        .tuples()
        .first()
    )

    # Top artists: plays, not distinct titles (Last.fm's own chart counts
    # scrobbles), grouped and ordered by the database in one statement.
    artist_rows = list(
        ListenEvent.select(
            Artist.id,
            Artist.deezer_id,
            Artist.name,
            fn.COUNT(ListenEvent.id).alias("plays"),
        )
        .join(Track, on=(ListenEvent.track == Track.id))
        .join(Artist, on=(Track.artist == Artist.id))
        .where(window & (ListenEvent.outcome > 0))
        .group_by(Artist.id, Artist.deezer_id, Artist.name)
        .order_by(fn.COUNT(ListenEvent.id).desc(), Artist.id)
        .limit(TOP)
        .tuples()
    )

    # Top titles: counted in one grouped statement, then payloads in one
    # batched query — the same shape every other list has.
    track_rows = list(
        ListenEvent.select(ListenEvent.track, fn.COUNT(ListenEvent.id).alias("plays"))
        .where(window & (ListenEvent.outcome > 0))
        .group_by(ListenEvent.track)
        .order_by(fn.COUNT(ListenEvent.id).desc(), ListenEvent.track)
        .limit(TOP)
        .tuples()
    )
    tracks = _track_payloads([tid for tid, _ in track_rows])
    top_tracks = [
        {"track": tracks[str(tid)], "count": int(plays)}
        for tid, plays in track_rows
        if str(tid) in tracks
    ]

    return {
        "days": days,
        "listens": int(listens or 0),
        "skips": int(skips or 0),
        "minutes": int((seconds or 0) // 60),
        "top_artists": [
            {
                "id": _uuid_text(aid),
                "name": name,
                "cover": f"/api/cover/{_uuid_text(aid)}" if deezer_id else None,
                "count": int(plays),
            }
            for aid, deezer_id, name, plays in artist_rows
        ],
        "top_tracks": top_tracks,
    }


@webapi.route("/history")
@login_required
def listening_history():
    """The caller's own listening log, newest first, with a 30-day summary."""
    try:
        limit = _limit_arg()
        days = _days_arg()
        outcome = _outcome_arg()
        before, before_id = _cursor()
    except ValueError as exc:
        return _bad(str(exc))

    # One row more than the page tells us whether a next one exists WITHOUT a
    # count statement; the cursor is then the last SERVED event, never the
    # probe, so the next page starts exactly where this one stopped.
    rows = list(
        _events_query(request.webuser, before, before_id, outcome).limit(limit + 1).tuples()
    )
    page = rows[:limit]
    tracks = _track_payloads([row[5] for row in page])
    events = [e for e in (_event_payload(row, tracks) for row in page) if e is not None]

    following = None
    if rows[limit:]:
        last = page[-1]
        following = {"before": last[1].isoformat(), "before_id": int(last[0])}

    return jsonify(
        {
            "events": events,
            "next": following,
            "summary": _summary(request.webuser, days),
        }
    )
