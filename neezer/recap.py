# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""One person's year in review, summed up from their own listening log.

``listen_event`` (``neezer/listening.py``) already holds every track change
with the seconds actually played and the published verdict (+1 a listen, -1 a
skip, 0 neither). This module only READS it: two grouped queries and a little
Python, no schema, no worker, no network.

**What the numbers mean** (the definitions the screens publish, so no two
screens can disagree):

* ``seconds`` is the time actually spent: every event's ``listened``, skips
  included — the same rule as ``/api/history``'s summary. ``minutes`` is that
  figure floored to the minute.
* ``listens`` / ``skips`` are ``outcome > 0`` / ``outcome < 0``; ``neutrals``
  is what remains. Tops count LISTENS only (a skip is not a taste), and so do
  the styles.
* The window is the SERVER's local calendar, half-open:
  ``[Y-01-01T00:00:00, (Y+1)-01-01T00:00:00)``. An event stamped exactly at
  midnight on 1 January belongs to the new year and never to the old one. No
  UTC conversion: this is the same clock ``/api/history`` reads.
* The year in progress is ``partial``: ``period.to`` is today, never
  31 December.

The thresholds are deliberately low because this server is one person's own
archive, not a global service:

======================  ==================================================
``MIN_YEAR``              the oldest year a link may name (2000)
``ENOUGH_LISTENS``        20 listens
``ENOUGH_MINUTES``        30 minutes of music
``enough``                either of the two, because a few very long titles
                          are an honest year too — but never without one
                          LISTEN: a year of skipped titles is not "ton année"
                          (plan §2.4), whatever the time spent
``STYLE_MIN_CONF``        the verdict must be sure (0.4, the same bar
                          ``deezer/explorer.ACCEPT_CONF`` applies)
``GENRE_MIN_COVERAGE``    a third of the listens measured, or no styles card
``STREAK_MIN``            three consecutive days is a story; two is not
======================  ==================================================

**Genres are the only part that could lie**, so they are locked down: the
single source is ``track_analysis.style`` (the measured verdict, never a
file's or a catalogue's tag, never a mood), a title counts only when its
confidence reaches ``STYLE_MIN_CONF``, and every style is carried up to its
meaningful parent by ``discover.lift_style`` — the exact function the
Explorer uses, not a second implementation. A style the taxonomy does not
know counts as unknown: a name the tree cannot place is never displayed. The
card always states its coverage; under a third it does not exist at all
rather than being replaced by "eclectic".

**Tops** are ordered by number of listens, then seconds descending, then id
ascending — a stable order, so the same log always paints the same five rows.
Artists are summed over their titles' listens (the track's PRIMARY artist, as
``/api/history`` does) and an artist's ``tracks`` is the number of distinct
titles heard. Albums likewise.

**Visibility**: every aggregate joins ``Track`` and applies
``Track.visible_clause`` — an upload that is no longer the caller's leaves
the totals, not just a list. A deleted title takes its events with it
(CASCADE), so no event is ever orphaned.

**Cost**: one statement for the days, one for the titles, then the route's
own batched payload (and its credits). A year with no event short-circuits
after the first statement. The process cache holds the AGGREGATES only, never
a track payload: a file archived between two views must change its quality
badge at once. 60 s for the year in progress, an hour for a finished one, 32
entries at most, keyed by person so nothing is ever shared between accounts.

No writes, no background work, and nothing here talks to a provider.
"""

from __future__ import annotations

import datetime
import threading
import time
from collections import OrderedDict

import peewee
from peewee import fn

from . import genretree
from .db import Album, Artist, ListenEvent, Track, TrackAnalysis, now
from .discover import lift_style

#: The oldest year a link may name.
MIN_YEAR = 2000
#: How much a year must hold to be worth telling: 20 listens OR 30 minutes.
ENOUGH_LISTENS = 20
ENOUGH_MINUTES = 30
#: A style is named only when its verdict is sure. Same bar as the Explorer's
#: ``deezer/explorer.ACCEPT_CONF`` (kept a literal: importing the deezer tree
#: from here would make a read of the log depend on the provider package).
STYLE_MIN_CONF = 0.4
#: Below this share of the listens measured, the styles card is not offered.
GENRE_MIN_COVERAGE = 0.34
#: Days in a row before the card is a story.
STREAK_MIN = 3
#: Rows per top list.
TOP = 5
#: The clock every bound is expressed in.
TIMEZONE = "server-local"

#: The current year still moves with every play; a finished one barely does.
TTL_CURRENT = 60
TTL_PAST = 3600
#: One entry per (person, year): a family server's whole history fits.
MAX_CACHED = 32


# -- normalisation -----------------------------------------------------------
# The three engines do not answer these two aggregates in the same Python
# types: ``SUM`` is a Decimal on MySQL and may be None for an empty set,
# ``DATE()`` is a string on SQLite and a date on PostgreSQL/MySQL, and an
# untyped ``MIN(datetime)`` comes back as a string on SQLite. Normalise once,
# here, rather than at every read.


def _as_int(value) -> int:
    """``COUNT``/``SUM`` as an int; ``None`` (an empty sum) is zero."""
    return int(value or 0)


def _as_date(value) -> datetime.date:
    """A ``DATE()`` group key as a date, whatever the engine returned."""
    if isinstance(value, datetime.datetime):
        return value.date()
    if isinstance(value, datetime.date):
        return value
    return datetime.date.fromisoformat(str(value)[:10])


def _as_datetime(value):
    """An untyped ``MIN``/``MAX(at)`` as a naive datetime, or None."""
    if value is None:
        return None
    if isinstance(value, datetime.datetime):
        return value
    return datetime.datetime.fromisoformat(str(value))


# -- the two aggregates ------------------------------------------------------


def _window(user, start, end):
    """``user``'s events in ``[start, end)``: one person, server-local clock."""
    return (ListenEvent.user == user) & (ListenEvent.at >= start) & (ListenEvent.at < end)


def _visible(query, user):
    """Apply the caller's track visibility. Never ``& True``: for an admin the
    clause is None, and a Python bool is not a SQL node."""
    clause = Track.visible_clause(user)
    return query if clause is None else query.where(clause)


def daily_query(user, start, end):
    """``(day, events, listens, skips, seconds)`` per calendar day, in ``[start,
    end)``.

    ``DATE(at)`` is spelled the same on SQLite, PostgreSQL and MySQL and reads
    the naive value that was stored: no ``strftime``, no ``date_trunc``, no
    ``DATE_FORMAT``, so the three engines answer the same day. The join to
    ``Track`` is what applies the caller's visibility to the TOTALS, not only
    to the lists.
    """
    day = fn.DATE(ListenEvent.at)
    query = (
        ListenEvent.select(
            day.alias("day"),
            fn.COUNT(ListenEvent.id),
            fn.SUM(peewee.Case(None, [(ListenEvent.outcome > 0, 1)], 0)),
            fn.SUM(peewee.Case(None, [(ListenEvent.outcome < 0, 1)], 0)),
            fn.SUM(ListenEvent.listened),
        )
        .join(Track, on=(ListenEvent.track == Track.id))
        .where(_window(user, start, end))
        .group_by(day)
        .order_by(day)
    )
    return _visible(query, user)


def track_query(user, start, end):
    """One row per title heard in ``[start, end)``: the title's own facts, its
    measured style, and its counts.

    Every grouped column is listed in ``group_by``: MySQL's
    ``ONLY_FULL_GROUP_BY`` must not have to infer a functional dependency
    across three joins. The analysis is a ``LEFT`` join (its ``track`` is the
    primary key, so at most one row and never a multiplied event), because a
    title nobody analysed still carries its listens — they are what makes the
    genre card's coverage honest.
    """
    query = (
        ListenEvent.select(
            ListenEvent.track,
            Track.artist,
            Track.album,
            Artist.name,
            Artist.deezer_id,
            Album.name,
            TrackAnalysis.style,
            TrackAnalysis.style_confidence,
            fn.COUNT(ListenEvent.id),
            fn.SUM(peewee.Case(None, [(ListenEvent.outcome > 0, 1)], 0)),
            fn.SUM(peewee.Case(None, [(ListenEvent.outcome < 0, 1)], 0)),
            fn.SUM(ListenEvent.listened),
            fn.MIN(peewee.Case(None, [(ListenEvent.outcome > 0, ListenEvent.at)], None)),
            fn.MAX(peewee.Case(None, [(ListenEvent.outcome > 0, ListenEvent.at)], None)),
        )
        .join(Track, on=(ListenEvent.track == Track.id))
        .join(Artist, on=(Track.artist == Artist.id))
        .join(Album, on=(Track.album == Album.id))
        .left_outer_join(TrackAnalysis, on=(TrackAnalysis.track == Track.id))
        .where(_window(user, start, end))
        .group_by(
            ListenEvent.track,
            Track.artist,
            Track.album,
            Artist.name,
            Artist.deezer_id,
            Album.name,
            TrackAnalysis.style,
            TrackAnalysis.style_confidence,
        )
    )
    return _visible(query, user)


def _daily_rows(user, start, end) -> list[tuple]:
    return [
        (_as_date(day), _as_int(events), _as_int(listens), _as_int(skips), _as_int(seconds))
        for day, events, listens, skips, seconds in daily_query(user, start, end).tuples()
    ]


def _track_rows(user, start, end) -> list[dict]:
    rows = []
    for (
        tid,
        artist_id,
        album_id,
        artist_name,
        artist_deezer_id,
        album_name,
        style,
        confidence,
        events,
        listens,
        skips,
        seconds,
        first_at,
        last_at,
    ) in track_query(user, start, end).tuples():
        rows.append(
            {
                "id": tid,
                "artist_id": artist_id,
                "album_id": album_id,
                "artist": artist_name,
                "artist_deezer_id": str(artist_deezer_id) if artist_deezer_id else None,
                "album": album_name,
                "style": style,
                "style_confidence": float(confidence or 0.0),
                "events": _as_int(events),
                "listens": _as_int(listens),
                "skips": _as_int(skips),
                "seconds": _as_int(seconds),
                "first": _as_datetime(first_at),
                "last": _as_datetime(last_at),
            }
        )
    return rows


# -- derived shapes ----------------------------------------------------------


def _months() -> list[dict]:
    """Twelve entries, zero-filled, in calendar order — always."""
    return [{"month": month, "listens": 0, "skips": 0, "seconds": 0} for month in range(1, 13)]


def _streak(days: list[datetime.date]):
    """The longest run of consecutive days, with its first and last day.

    Clamped to the year by construction: only the days of THIS year are in
    ``days``, so a run across 31 December is counted in each year for its own
    part. ``None`` when no day was listened to.
    """
    best = 0
    best_from = best_to = None
    run = 0
    run_from = None
    previous = None
    for day in days:
        if previous is not None and day == previous + datetime.timedelta(days=1):
            run += 1
        else:
            run = 1
            run_from = day
        if run > best:
            best, best_from, best_to = run, run_from, day
        previous = day
    if not best:
        return None
    return {"days": best, "from": best_from.isoformat(), "to": best_to.isoformat()}


def _biggest_day(rows: list[tuple]):
    """Most listens in a day; a tie goes to more seconds, then the earlier day."""
    days = [(day, listens, seconds) for day, _e, listens, _s, seconds in rows if listens > 0]
    if not days:
        return None
    day, listens, seconds = max(days, key=lambda r: (r[1], r[2], -r[0].toordinal()))
    return {"date": day.isoformat(), "listens": listens, "seconds": seconds}


def _top_tracks(rows: list[dict]) -> list[dict]:
    """By listens, then seconds, then id — never a skipped-only title."""
    heard = [r for r in rows if r["listens"] > 0]
    heard.sort(key=lambda r: (-r["listens"], -r["seconds"], str(r["id"])))
    return [
        {"id": r["id"], "listens": r["listens"], "seconds": r["seconds"]}
        for r in heard[:TOP]
    ]


def _top_artists(rows: list[dict]) -> list[dict]:
    """Each artist's titles' listens summed (Track.artist, the primary one)."""
    by_artist: dict = {}
    for row in rows:
        if row["listens"] <= 0:
            continue
        got = by_artist.get(row["artist_id"])
        if got is None:
            got = by_artist[row["artist_id"]] = {
                "id": row["artist_id"],
                "name": row["artist"],
                "deezer_id": row["artist_deezer_id"],
                "listens": 0,
                "tracks": set(),
            }
        got["listens"] += row["listens"]
        got["tracks"].add(row["id"])
    ordered = sorted(
        by_artist.values(), key=lambda a: (-a["listens"], a["name"], str(a["id"]))
    )
    return [
        {
            "id": a["id"],
            "name": a["name"],
            "deezer_id": a["deezer_id"],
            "listens": a["listens"],
            "tracks": len(a["tracks"]),
        }
        for a in ordered[:TOP]
    ]


def _top_albums(rows: list[dict]) -> list[dict]:
    """Each album's titles' listens summed, with the title's primary artist."""
    by_album: dict = {}
    for row in rows:
        if row["listens"] <= 0:
            continue
        got = by_album.get(row["album_id"])
        if got is None:
            got = by_album[row["album_id"]] = {
                "id": row["album_id"],
                "name": row["album"],
                "artist": row["artist"],
                "listens": 0,
            }
        got["listens"] += row["listens"]
    ordered = sorted(by_album.values(), key=lambda a: (-a["listens"], a["name"], str(a["id"])))
    return [
        {"id": a["id"], "name": a["name"], "artist": a["artist"], "listens": a["listens"]}
        for a in ordered[:TOP]
    ]


def _genres(rows: list[dict], listens: int) -> dict:
    """The measured styles, carried up to their meaningful parent.

    Only a title whose verdict is sure AND whose style the taxonomy can place
    contributes. The card is offered only above ``GENRE_MIN_COVERAGE``, and
    always carries that coverage: the screen says on what share of the listens
    it is speaking.
    """
    tree = genretree.get() if listens else None
    measured: dict = {}
    measured_listens = 0
    for row in rows:
        if row["listens"] <= 0 or not row["style"]:
            continue
        if row["style_confidence"] < STYLE_MIN_CONF:
            continue
        node = tree.node(row["style"]) if tree is not None else None
        if node is None:
            continue  # a label the taxonomy does not know: never displayed
        target = lift_style(tree, node)
        if not target:
            continue
        got = measured.setdefault(target, {"name": target, "listens": 0})
        got["listens"] += row["listens"]
        measured_listens += row["listens"]
    coverage = (measured_listens / listens) if listens > 0 else 0.0
    ordered = sorted(measured.values(), key=lambda g: (-g["listens"], g["name"]))[:TOP]
    for row in ordered:
        row["share"] = round(row["listens"] / measured_listens, 4) if measured_listens else 0.0
        row["family"] = tree.family(row["name"]) if tree is not None else None
    return {
        "available": bool(ordered) and coverage >= GENRE_MIN_COVERAGE,
        "coverage": round(coverage, 4),
        "measured_listens": measured_listens,
        "explored": len(measured),
        "rows": ordered,
    }


def _zero_genres() -> dict:
    """The styles block of a year with nothing measured (no tree read)."""
    return {
        "available": False,
        "coverage": 0.0,
        "measured_listens": 0,
        "explored": 0,
        "rows": [],
    }


# -- one year ----------------------------------------------------------------


def _compute_year(user, year: int) -> dict:
    current = now()
    today = current.date()
    start = datetime.datetime(year, 1, 1)
    end = datetime.datetime(year + 1, 1, 1)
    partial = year == current.year
    period = {
        "from": start.date().isoformat(),
        "to": (today if partial else end.date() - datetime.timedelta(days=1)).isoformat(),
    }

    daily = _daily_rows(user, start, end)
    if not daily:
        # A year with no event is an EMPTY STATE, not an error and not work:
        # the second aggregate is never run.
        return {
            "year": year,
            "partial": partial,
            "enough": False,
            "timezone": TIMEZONE,
            "period": period,
            "totals": {
                "events": 0,
                "listens": 0,
                "skips": 0,
                "neutrals": 0,
                "seconds": 0,
                "minutes": 0,
                "days_active": 0,
                "first_at": None,
                "last_at": None,
            },
            "months": _months(),
            "streak": None,
            "biggest_day": None,
            "tops": {
                "tracks": [],
                "artists": [],
                "albums": [],
                "genres": _zero_genres(),
            },
            "first": None,
            "last": None,
            "cards": [],
            "generated": int(time.time()),
        }

    events = sum(r[1] for r in daily)
    listens = sum(r[2] for r in daily)
    skips = sum(r[3] for r in daily)
    seconds = sum(r[4] for r in daily)
    minutes = seconds // 60

    months = _months()
    for day, _events, day_listens, day_skips, day_seconds in daily:
        month = months[day.month - 1]
        month["listens"] += day_listens
        month["skips"] += day_skips
        month["seconds"] += day_seconds

    days_listened = sorted({day for day, _e, day_listens, _s, _sec in daily if day_listens > 0})
    streak = _streak(days_listened)
    biggest_day = _biggest_day(daily)

    rows = _track_rows(user, start, end)
    tops_tracks = _top_tracks(rows)
    tops_artists = _top_artists(rows)
    tops_albums = _top_albums(rows)
    genres = _genres(rows, listens)

    heard_first = sorted(
        (r for r in rows if r["first"] is not None),
        key=lambda r: (r["first"], str(r["id"])),
    )
    heard_last = sorted(
        (r for r in rows if r["last"] is not None),
        key=lambda r: (r["last"], str(r["id"])),
    )
    first = (
        {"id": heard_first[0]["id"], "at": heard_first[0]["first"].isoformat()}
        if heard_first
        else None
    )
    last = (
        {"id": heard_last[-1]["id"], "at": heard_last[-1]["last"].isoformat()}
        if heard_last
        else None
    )

    enough = listens > 0 and (listens >= ENOUGH_LISTENS or minutes >= ENOUGH_MINUTES)
    cards = []
    if enough:
        cards.append("year")
        cards.append("minutes")
        if tops_artists:
            cards.append("artists")
        if tops_tracks:
            cards.append("tracks")
        if tops_albums:
            cards.append("albums")
        if genres["available"]:
            cards.append("genres")
        if streak is not None and streak["days"] >= STREAK_MIN:
            cards.append("streak")
        if first is not None and last is not None:
            cards.append("firstlast")

    return {
        "year": year,
        "partial": partial,
        "enough": enough,
        "timezone": TIMEZONE,
        "period": period,
        "totals": {
            "events": events,
            "listens": listens,
            "skips": skips,
            "neutrals": events - listens - skips,
            "seconds": seconds,
            "minutes": minutes,
            "days_active": len(days_listened),
            "first_at": first["at"] if first else None,
            "last_at": last["at"] if last else None,
        },
        "months": months,
        "streak": streak,
        "biggest_day": biggest_day,
        "tops": {
            "tracks": tops_tracks,
            "artists": tops_artists,
            "albums": tops_albums,
            "genres": genres,
        },
        "first": first,
        "last": last,
        "cards": cards,
        # Frozen with the aggregate so two views inside the TTL are the same
        # bytes and the client gets an empty 304.
        "generated": int(time.time()),
    }


# -- every year with events --------------------------------------------------


def _compute_years(user) -> dict:
    """The years that hold at least one event, newest first.

    Deliberately NOT floored at ``listening.KEEP_DAYS``: a restored server (or
    a prune that has not run yet) may hold an older row, and a year the year
    route can serve must appear as a chip. One statement; the grouping by
    year happens in Python on at most a few hundred day rows.
    """
    current = now()
    start = datetime.datetime(MIN_YEAR, 1, 1)
    # Half-open and inclusive of the second ``now()`` was taken in, so a play
    # landing while this request runs is never lost from the list it just
    # joined.
    end = current + datetime.timedelta(seconds=1)

    by_year: dict = {}
    for day, events, listens, _skips, seconds in _daily_rows(user, start, end):
        got = by_year.setdefault(day.year, [0, 0, 0])
        got[0] += events
        got[1] += listens
        got[2] += seconds

    years = []
    for year in sorted(by_year, reverse=True):
        events, listens, seconds = by_year[year]
        minutes = seconds // 60
        years.append(
            {
                "year": year,
                "partial": year == current.year,
                "enough": (
                    listens > 0
                    and (listens >= ENOUGH_LISTENS or minutes >= ENOUGH_MINUTES)
                ),
                "events": events,
                "listens": listens,
                "minutes": minutes,
            }
        )
    return {
        "years": years,
        "latest": years[0]["year"] if years else None,
        "now_year": current.year,
        "min_year": MIN_YEAR,
        "timezone": TIMEZONE,
    }


# -- process cache -----------------------------------------------------------
# Aggregates only. A track payload is assembled on every request, from the
# rows as they are NOW: an archive that finished between two views changes the
# quality badge immediately, and a title that disappeared leaves the lists.
# Keyed by person, so nothing here can ever answer for somebody else.

_CACHE: "OrderedDict[tuple, tuple]" = OrderedDict()
_YEARS: "OrderedDict[str, tuple]" = OrderedDict()
_LOCK = threading.RLock()


def _cache_get(cache, key):
    with _LOCK:
        entry = cache.get(key)
        if entry is None:
            return None
        expires_at, value = entry
        if expires_at <= time.monotonic():
            cache.pop(key, None)
            return None
        cache.move_to_end(key)
        return value


def _cache_put(cache, key, value, ttl: float) -> None:
    with _LOCK:
        cache[key] = (time.monotonic() + ttl, value)
        cache.move_to_end(key)
        while len(cache) > MAX_CACHED:
            cache.popitem(last=False)


def _user_key(user) -> str:
    return str(getattr(user, "id", user))


def invalidate(user=None) -> None:
    """Drop every cached aggregate, or only ``user``'s.

    For tests, and for a future call from the listen route (not wired: the
    60 s TTL of the current year is short enough).
    """
    with _LOCK:
        if user is None:
            _CACHE.clear()
            _YEARS.clear()
            return
        uid = _user_key(user)
        for key in [k for k in _CACHE if k[0] == uid]:
            _CACHE.pop(key, None)
        _YEARS.pop(uid, None)


def year_stats(user, year: int) -> dict:
    """The aggregated year for ``user``, from the cache or the database.

    The returned dict is CACHED and shared: treat it as read-only. The route
    builds its own payload and only ever reads track ids from here.
    """
    year = int(year)
    key = (_user_key(user), year)
    cached = _cache_get(_CACHE, key)
    if cached is not None:
        return cached
    value = _compute_year(user, year)
    _cache_put(_CACHE, key, value, TTL_CURRENT if year == now().year else TTL_PAST)
    return value


def years(user) -> dict:
    """``{years, latest, now_year, min_year, timezone}`` for the year chips."""
    key = _user_key(user)
    cached = _cache_get(_YEARS, key)
    if cached is not None:
        return cached
    value = _compute_years(user)
    _cache_put(_YEARS, key, value, TTL_CURRENT)
    return value
