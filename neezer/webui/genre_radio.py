# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Genre stations: a continuous radio anchored on one genre of the tree.

The routes register themselves on the shared ``webapi`` blueprint (imported at
the bottom of ``neezer/webui/__init__.py``), so they answer under ``/api`` with
the same session cookie and same-origin guard as every other route.

* ``GET  /api/radio/genre/<style>``  -> ``{style, name, tracks}``
* ``POST /api/radio/genre/next``     -> ``{style, name, tracks}``

A station is STYLE-FIRST: it plays the titles the engine placed in the style
and in everything under it, and nothing else. Inside that branch it is the
person's own station — their taste, their skips and their discovery level cut
down to the style by ``discover.style_taste`` — and it is assembled by the same
machinery as the Flow (``discover.flow``), so a refill varies instead of
replaying the same order.

Every read comes from the database, like the Flow and the radio: a click never
asks a provider, and a library whose Deezer account is unreachable still starts
a station. The station is its OWN playback context — it passes the style in the
request and never writes ``UserTaste`` — so it cannot overwrite the Flow's
saved styles or energy notch.
"""

import secrets

from flask import jsonify, request

from .. import discover, genretree
from ..db import TrackAnalysis
from . import _ref_list, login_required, webapi
from .discover import FLOW_SIZE, _level_arg, _serialise_tracks

#: Titles a station answers with, the same row as a Flow answer.
STATION_SIZE = FLOW_SIZE
#: What a refill's body may carry, at most (a radio's own caps).
RECENT_MAX = 40
SKIPPED_MAX = 30
EXCLUDE_MAX = 400


def _node(name):
    """The tree node a client-supplied style stands for, or ``None``. Names are
    the tree's own (spaces and accents included); a folded spelling or an
    engine family id resolves too, so an old client's label keeps working."""
    return genretree.get().node(str(name or "")[:64])


def _level(data=None):
    """The discovery breadth: the body's ``d`` (a refill), else the ``?d=`` of
    a GET, else the person's stored level — never a write."""
    if data is not None and data.get("d") not in (None, ""):
        return discover.clamp(data.get("d"))
    return _level_arg()


def _in_branch(cands, node):
    """Only the titles the engine placed in ``node`` or under it.

    ``discover.flow`` draws from a wider pool than the branch: a sound
    neighbour of a loved title, or a catalog find the explorer verified only
    NEAR the style. A station promises one genre, so those are dropped here.
    A title with no verdict cannot be said to be in the style either."""
    ids = [c.track for c in cands if c.track is not None]
    styles = {}
    if ids:
        styles = dict(
            TrackAnalysis.select(TrackAnalysis.track, TrackAnalysis.style)
            .where(TrackAnalysis.track.in_(ids))
            .tuples()
        )
    out = []
    for c in cands:
        if c.track is None:
            continue
        style = styles.get(c.track)
        if style and genretree.within(style, node):
            out.append(c)
    return out


def _station(user, node, level, seed=None, exclude=(), recent=(), skipped=()):
    """The station's titles: the Flow's draw around one style, cut to the
    branch. ``recent`` is folded into ``exclude`` — a station's refill must
    never hand back a title the queue already played, whether the client
    called it "queued" or "recent"."""
    picked = discover.flow(
        user, level, STATION_SIZE,
        seed=seed or secrets.token_hex(4),
        exclude=set(exclude) | set(recent),
        skipped=skipped,
        styles=(node,),
    )
    return _in_branch(picked, node)


def _payload(node, cands):
    """``{style, name, tracks}``: ``style`` and ``name`` are the tree's
    canonical node name — the exact string a refill POST takes back."""
    return jsonify({"style": node, "name": node, "tracks": _serialise_tracks(cands)})


@webapi.route("/radio/genre/<path:style>")
@login_required
def genre_radio(style):
    """Start a station on a genre. ``<path:style>``: a genre name may hold a
    slash ("Disco / funk"), which the client URL-encodes."""
    node = _node(style)
    if node is None:
        return jsonify({"error": "unknown style"}), 404
    try:
        level = _level()
    except ValueError:
        return jsonify({"error": "invalid level"}), 400
    return _payload(node, _station(request.webuser, node, level))


@webapi.route("/radio/genre/next", methods=["POST"])
@login_required
def genre_radio_next():
    """The next titles of a playing station. Body: ``style``, ``d`` (optional
    discovery breadth), ``recent`` / ``skipped`` (what this session heard and
    passed) and ``exclude`` (what the queue holds). Nothing is persisted: the
    station is a playback context, not the Flow's saved aim."""
    data = request.get_json(silent=True)
    if not isinstance(data, dict):
        return jsonify({"error": "invalid body"}), 400
    node = _node(data.get("style"))
    if node is None:
        return jsonify({"error": "unknown style"}), 404
    try:
        level = _level(data)
    except ValueError:
        return jsonify({"error": "invalid level"}), 400
    recent = _ref_list(data, "recent", RECENT_MAX)
    skipped = _ref_list(data, "skipped", SKIPPED_MAX)
    exclude = _ref_list(data, "exclude", EXCLUDE_MAX)
    return _payload(
        node,
        _station(request.webuser, node, level, exclude=exclude, recent=recent, skipped=skipped),
    )
