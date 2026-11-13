"""Mood mixes built from what the server already measured.

A mood is a set of measured bands (energy, tempo, spectral colour) over the
library, never a hand-typed tag: ``neezer/moods.py`` holds the definitions and
the assembly, this module exposes them.

Contract:

* ``GET /api/moods``            -> ``{moods: [{id, name, subtitle, tracks, ready, cover}]}``
* ``GET /api/moods/<id>``       -> ``{id, name, subtitle, tracks, ready}``

The routes register themselves on the shared ``webapi`` blueprint, imported at
the bottom of ``neezer/webui/__init__.py``. No provider is asked: a mood is
made from archived files whose analysis the server already holds.
"""

from flask import jsonify, request

from .. import moods
from ..db import Album, Artist, Track
from . import _db_track_rows, login_required, webapi
from .refs import cover_url_for


def _payload(ids):
    """The API dicts of ``ids`` (row UUIDs), in that order, in one read.

    ``_db_track_rows`` serialises a whole list from tuples and resolves every
    credit in one more statement — the same path the favourites and playlist
    lists take, never a query per row. A row deleted between the catalog cache
    and this call is simply left out.
    """
    if not ids:
        return []
    rows = _db_track_rows(
        Track.select(Track, Album, Artist)
        .join(Album)
        .switch(Track)
        .join(Artist)
        .where(Track.id.in_(list(ids)))
    )
    by_id = {row["id"]: row for row in rows}
    return [by_id[str(tid)] for tid in ids if str(tid) in by_id]


@webapi.route("/moods")
@login_required
def mood_mixes():
    """The six moods with their matching count and their card's cover.

    One catalog read per person per day (cached a minute in the process): the
    card row costs no query per card, and the cover is the first title's own
    album, so the page it opens shows the same sleeve first.
    """
    catalog = moods.catalog(request.webuser)
    out = []
    for entry in catalog.entries():
        out.append({
            "id": entry.mood.id,
            "name": entry.mood.name,
            "subtitle": entry.mood.subtitle,
            "tracks": entry.tracks,
            "ready": entry.ready,
            "cover": cover_url_for(entry.cover_album) if entry.cover_album else None,
        })
    return jsonify({"moods": out})


@webapi.route("/moods/<mood_id>")
@login_required
def mood_mix(mood_id):
    """One mood's mix, at most ``limit`` titles (40 by default and at most).

    An unknown id is a 404 before any work. A mood with too few matching
    titles answers ``tracks: []`` and ``reason: "not-enough"`` — the card is
    not offered, and a stale link says why instead of playing two songs.
    """
    if moods.definition(mood_id) is None:
        return jsonify({"error": "unknown mood"}), 404
    raw = request.args.get("limit")
    if raw in (None, ""):
        limit = moods.LIST_MAX
    else:
        try:
            limit = int(raw)
        except (TypeError, ValueError):
            return jsonify({"error": "invalid limit"}), 400
    limit = max(1, min(limit, moods.LIST_MAX))
    catalog = moods.catalog(request.webuser)
    entry = catalog.get(mood_id)
    if entry is None:  # a definition the catalog did not build (never, today)
        return jsonify({"error": "unknown mood"}), 404
    out = {
        "id": mood_id,
        "name": entry.mood.name,
        "subtitle": entry.mood.subtitle,
        "tracks": [],
        "ready": entry.ready,
    }
    if not entry.ready:
        out["reason"] = "not-enough"
        return jsonify(out)
    rows = _payload(catalog.mix_ids(mood_id, limit))
    out["tracks"] = rows
    if not rows:
        out["reason"] = "no-tracks"
    return jsonify(out)
