# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Recommendations for the bottom of a playlist.

The playlist itself is the seed: its titles' stored sound neighbours, their
artists and their styles feed the existing radio assembler, minus what the
playlist already holds. A long playlist is represented by an evenly spaced
sample of its titles (``MAX_SEEDS``), because the assembler reads the stored
neighbours of every seed it is given. The section answers "what next?" where a
playlist stops, without ever leaving the library: no click here asks a provider.

Contract:

* ``GET /api/playlist/<playlist_id>/recommendations?limit=12``
  -> ``{playlist: {id, title}, tracks: [...]}``

The playlist is resolved like every other playlist route, so a Deezer
editorial playlist (no row here) is a 404: there is nothing of ours to seed
from. The route is READ-ONLY; adding a suggestion stays the ordinary
``POST /api/playlist/<id>/tracks``.

The draw is seeded with the playlist's id and the day, so the suggestions do
not move while the page is open and are new tomorrow (the same rule as the
home's mixes). The days are the server's own, like every other date.
"""

from __future__ import annotations

import random
from datetime import date

from flask import jsonify, request

from .. import radio
from ..db import PlaylistTrack, Track
from . import _resolve_db_playlist, _tracks_in_order, login_required, webapi

#: Enough to fill the section under a phone; the SPA asks for this.
DEFAULT_LIMIT = 12
#: Never more than the section can show without becoming a second playlist.
MAX_LIMIT = 20
#: A playlist of 4 000 titles cannot be fed to the radio whole: ``radio.build``
#: reads the stored neighbours of every seed and of their best neighbours, and
#: measured on the 8 000-title bench a 4 000-title playlist took 2 171 ms
#: against 36 ms for an ordinary one. The seeds are therefore a deterministic,
#: evenly spaced SAMPLE of the playlist — the whole list still excludes itself
#: from the answers. At 80 seeds the same 4 000-title playlist answers in
#: 139 ms (12 statements), and every part of a long playlist is represented.
MAX_SEEDS = 80


def _limit() -> int:
    """``?limit=`` read defensively: anything unusable falls back to the
    default, and nothing can ask for a hundred suggestions."""
    raw = request.args.get("limit", DEFAULT_LIMIT)
    try:
        limit = int(raw)
    except (TypeError, ValueError):
        return DEFAULT_LIMIT
    if limit < 1:
        return DEFAULT_LIMIT
    return min(MAX_LIMIT, limit)


def _seed_sample(seed_ids):
    """The playlist's own ids, all of them when short enough, else an evenly
    spaced sample that keeps the first and the last (a playlist has an order,
    and its two ends must both count). Deterministic: the same playlist always
    seeds the same radio."""
    if len(seed_ids) <= MAX_SEEDS:
        return list(seed_ids)
    last = len(seed_ids) - 1
    return [seed_ids[round(i * last / (MAX_SEEDS - 1))] for i in range(MAX_SEEDS)]


@webapi.route("/playlist/<playlist_id>/recommendations")
@login_required
def playlist_recommendations(playlist_id):
    pl = _resolve_db_playlist(playlist_id)
    if pl is None:
        return jsonify({"error": "not found"}), 404

    # The playlist's own rows are the seeds. One statement, and the duplicate
    # occurrences a playlist may hold collapse to one seed: the same title
    # twice says nothing more about what the playlist is.
    seed_ids = list(
        dict.fromkeys(
            r[0]
            for r in Track.select(Track.id)
            .join(PlaylistTrack, on=(PlaylistTrack.track == Track.id))
            .where(PlaylistTrack.playlist == pl)
            .order_by(PlaylistTrack.index)
            .tuples()
        )
    )
    payload = {"playlist": {"id": str(pl.id), "title": pl.name}}
    if len(seed_ids) < 2:
        # One title is not a playlist to extrapolate from: an honest empty
        # answer, not an error and not a radio pretending to be a suggestion.
        return jsonify({**payload, "tracks": []})

    rng = random.Random(  # nosec B311 - a draw for variety, not a secret
        f"{pl.id}:{date.today().isoformat()}"
    )
    # ``radio.build`` already drops what is unavailable, hidden from this
    # person, skipped twice, disliked or heard in the last 20 minutes; the
    # playlist's own titles are excluded both as candidates and as seeds. The
    # sample only limits how many SEEDS the radio reads (MAX_SEEDS above): every
    # row of the playlist is still excluded from the answer.
    picks = radio.build(
        request.webuser,
        _seed_sample(seed_ids),
        n=_limit(),
        exclude=seed_ids,
        rng=rng,
        block_seeds=True,
    )
    return jsonify({**payload, "tracks": _tracks_in_order(picks)})
