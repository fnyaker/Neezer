# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""The Récap's two reads: the years that hold a history, and one year in review.

Contract:

* ``GET /api/recap/years`` -> ``{years, latest, now_year, min_year, timezone}``
* ``GET /api/recap/<year>`` -> the full year, or ``400 {"error": "invalid
  year"}`` for a year before 2000 or after the server's current one.

The aggregation itself lives in ``neezer/recap.py`` (pure: the database and the
genre tree, no Flask). This module only serialises: the standard track payload
comes from ``_db_track_rows(_visible_tracks(...))`` for the titles the tops and
the first/last cards name — never a home-made dict — in one statement plus the
credits'. The artist and album cards carry the same cover expression as the
rest of the API, so a cover is never a provider call.

**Privacy is not a parameter.** Both routes read ``request.webuser`` and
nothing else: there is no ``?user=``, and an admin gets their own year like
everybody else (the same rule as ``/api/history``). A year is never written,
never cached on the device by this server, and a title the caller may no longer
see leaves the totals with the lists (the aggregates apply
``Track.visible_clause``).

**Nothing here calls a provider, writes a row, or starts a worker.** A year
costs at most four statements (see ``neezer/recap.py``); an empty one, one.
"""

from __future__ import annotations

from flask import jsonify, request

from .. import recap
from ..db import Album, Artist, Track, now
from . import _db_track_rows, _visible_tracks, login_required, webapi
from .refs import cover_url_for


def _track_payloads(ids) -> dict:
    """The standard API dicts for ``ids``, keyed by the track UUID as text.

    One statement for the titles (read as tuples) and one for their credits:
    the same batched path every list uses, never one query per row. A title
    that is no longer visible is simply absent, and its card row is dropped.
    """
    ids = [i for i in ids if i is not None]
    if not ids:
        return {}
    rows = _db_track_rows(
        _visible_tracks(
            Track.select(Track, Album, Artist)
            .join(Album)
            .switch(Track)
            .join(Artist)
            .where(Track.id.in_(ids))
        )
    )
    return {row["id"]: row for row in rows}


def _edge(edge, tracks) -> dict | None:
    """A first/last card: the instant and the title as the rest of the API
    serves it, or None when the title is gone from the caller's library."""
    if not edge:
        return None
    track = tracks.get(str(edge["id"]))
    if track is None:
        return None
    return {"at": edge["at"], "track": track}


def _payload(stats) -> dict:
    """Serialise one aggregated year. Reads the cache, never mutates it."""
    tops = stats["tops"]
    payloads = _track_payloads(
        [row["id"] for row in tops["tracks"]]
        + ([stats["first"]["id"]] if stats["first"] else [])
        + ([stats["last"]["id"]] if stats["last"] else [])
    )
    top_tracks = [
        {
            "track": payloads[str(row["id"])],
            "listens": row["listens"],
            "seconds": row["seconds"],
        }
        for row in tops["tracks"]
        if str(row["id"]) in payloads
    ]
    artists = [
        {
            "id": str(artist["id"]),
            "name": artist["name"],
            # Same expression as the history summary: an artist has a cover
            # route only when the library knows its row.
            "cover": f"/api/cover/{artist['id']}" if artist["deezer_id"] else None,
            "listens": artist["listens"],
            "tracks": artist["tracks"],
        }
        for artist in tops["artists"]
    ]
    albums = [
        {
            "id": str(album["id"]),
            "title": album["name"],
            "artist": album["artist"],
            "cover": cover_url_for(album["id"]),
            "listens": album["listens"],
        }
        for album in tops["albums"]
    ]
    first = _edge(stats["first"], payloads)
    last = _edge(stats["last"], payloads)
    # The aggregate chose the cards; a title that became unreadable between
    # the cached read and this payload must not leave a card with nothing in
    # it. Only ever removes an id, never adds one.
    filled = {
        "artists": bool(artists),
        "tracks": bool(top_tracks),
        "albums": bool(albums),
        "firstlast": first is not None and last is not None,
    }
    cards = [card for card in stats["cards"] if filled.get(card, True)]
    return {
        "year": stats["year"],
        "partial": stats["partial"],
        "enough": stats["enough"],
        "timezone": stats["timezone"],
        "period": stats["period"],
        "totals": stats["totals"],
        "months": stats["months"],
        "streak": stats["streak"],
        "biggest_day": stats["biggest_day"],
        "tops": {
            "tracks": top_tracks,
            "artists": artists,
            "albums": albums,
            "genres": tops["genres"],
        },
        "first": first,
        "last": last,
        "cards": cards,
        "generated": stats["generated"],
    }


@webapi.route("/recap/years")
@login_required
def recap_years():
    """The caller's own years that hold at least one event, newest first."""
    return jsonify(recap.years(request.webuser))


@webapi.route("/recap/<int:year>")
@login_required
def recap_year(year):
    """One year of the caller's own listening; a year out of range is a 400."""
    if year < recap.MIN_YEAR or year > now().year:
        return jsonify({"error": "invalid year"}), 400
    return jsonify(_payload(recap.year_stats(request.webuser, year)))
