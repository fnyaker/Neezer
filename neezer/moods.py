# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Mood mixes ("mixes d'ambiance") from what the server already measured.

Every streaming service sells mood playlists built by people who listened to
the records; this server has no such editors, so a mood here is a **set of
measured bands** over the library and nothing else. The measures are the ones
``deezer/analysis.py`` already stores per archived file (``track_analysis``):

* ``energy`` — the 0..1 RANK built by ``neezer/energy.py`` from loudness,
  dynamic range, spectral centroid, flux, tempo and entropy (Spotify's own
  definition of energy, re-built from what we measure);
* ``bpm`` with its ``bpm_confidence`` — the beat-tracked tempo, never used
  below ``energy.BPM_TRUSTED``;
* ``centroid`` — the median spectral centroid (Hz), turned into a 0..1 rank
  **within this library** (colour: dark → bright);
* ``fluxPeak`` — how much the busiest frames stand out from the typical one
  (a scale-free ratio), also ranked within the library (steady → busy);
* ``style`` — the served verdict, used only as a *preference* for "Soirée".

WHY THESE SIX, AND WHY THOSE BANDS
----------------------------------
The streaming services organise by **mood and moment**, not by genre:
YouTube Music's four activities are Workout, Focus, Relax and Commute
(blog.youtube, 2020-11-09, "YouTube Music brings personalization to your
everyday moods and moments"); Apple Music shipped 250+ playlists whose groups
are Happy, Chill, Focus, Get Going/Party, Workout… (MacStories, 2021-10-22,
"A Comprehensive Guide to 250+ of Apple Music's New Mood and Activity
Playlists"); Spotify's daylist changes its title and titles through the day
(early morning, morning, afternoon, evening, night, late night) and leans on
microgenres (newsroom.spotify.com, 2023-09-12, "Get Fresh Music Sunup to
Sundown With daylist"). Six is the smallest set that covers the four YouTube
moments (Relax = Calme, Focus = Concentration, Workout = Énergie, Commute =
between Calme and Concentration) plus the two Apple/Spotify groups this
library can actually hear (Happy = Bonne humeur, party = Soirée) and the night
(Spotify's late-night dayparts).

The bands follow the two axes the literature agrees on. Tempo is regarded as
the most important structural feature behind music's emotional expression,
faster music raises arousal, louder music raises arousal, and mode/valence is
a separate axis (Wikipedia, "Music and emotion", §Structural features; the
circumplex model, Russell 1980, via "Emotion classification"). Energy and
tempo are also the two features whose relation to mood is straightforward
enough that a linear model beat an RBF one (Goel & Padial, Stanford CS229,
2011, "Music Mood Classification"). So:

===========  =============================  ==========  ==========  =======
mood          energy (rank)                  BPM         centroid    flux
===========  =============================  ==========  ==========  =======
Calme         <= 0.28                        45..105     >= 0.35      —
Concentration  0.24..0.58                    90..135     0.15..0.60  <= 0.45
Bonne humeur   0.32..0.68                    —           >= 0.62      —
Énergie       >= 0.72                        108..155    —            —
Soirée        >= 0.72                        118..200    —            —
Nuit          <= 0.28                        45..95      <= 0.35      —
===========  =============================  ==========  ==========  =======

The ranks make the bands portable: an 8 000-track library of chamber music and
one of hardcore both get a Calme and an Énergie, each holding its quieter and
louder quarter, because ``energy`` is a rank and centroid/flux are ranked here
the same way. The BPM bands are absolute because a tempo is a tempo. Calme and
Nuit share the bottom of the energy scale on purpose — Nuit takes the dark
side (low centroid rank), Calme the rest — and Énergie and Soirée share the
high one: Soirée only *prefers* dance/electronic verdicts when the analysis
has one, it never requires it. A mood with fewer than ``READY_MIN`` matching
titles is not offered at all rather than played as a two-track mix.

WHAT WE REFUSE TO FAKE
----------------------
* **instrumental** — nothing measures vocals. "Concentration" claims a steady,
  low-flux moderate tempo, never "sans paroles".
* **key / major-minor** — no chroma, no key estimate. Valence is therefore not
  measured, and "Bonne humeur" is a *proxy* (bright timbre + moderate energy),
  not a claim about joy. "Triste" is not offered for the same reason.
* **lyrics sentiment** — the words are not stored in a form we can judge.
* **genre as mood** — a style is a preference for Soirée only; the bands do the
  work, so a rock record at 130 BPM can be in Énergie and a dark ambient one in
  Nuit whatever Deezer calls them.
* **a title without the needed measure is never guessed**: no energy, no mix;
  a tempo below ``BPM_TRUSTED`` cannot satisfy a BPM band; a missing centroid
  cannot satisfy a brightness band.

COST
----
``catalog`` reads the analysis rows and the tracks they belong to in ONE joined
statement, reuses ``energy.energies()`` for the energy ranks, and keeps the
result ``TTL`` seconds per (person, day) in this process. Assembling a mix is
pure Python over that cached list; the route serialises only the 40 chosen
rows. No provider is asked, ever: Deezer being unreachable changes nothing
here.
"""

import bisect
import json
import logging
import math
import random
import threading
import time
import unicodedata
from dataclasses import dataclass

from . import discover, energy, mix
from .db import Track, TrackAnalysis, db, now

logger = logging.getLogger(__name__)

#: Fewer matching titles than this is not a mix worth offering.
READY_MIN = 8
#: The longest mood mix, and the default of the detail route.
LIST_MAX = 40
#: The opening title is drawn among this many best-fitting ones, so a mix does
#: not start on the same record every day while the card and the page still
#: agree (both use the same seed).
FIRST_POOL = 12
#: How long a catalog is served before it is rebuilt (seconds).
TTL = 60.0
#: Catalog entries kept in this process (each is one person's day).
MAX_CACHED = 16

#: Styles that make "Soirée" prefer a title. Family ids and labels the engine
#: can serve (``deezer/analysis.py#FAMILIES``), plus the words a trained head
#: commonly returns; matched as substrings of the folded style.
DANCE_STYLES = (
    "house", "techno", "trance", "dance", "disco", "dnb", "drum & bass",
    "drum and bass", "dubstep", "psytrance", "hardstyle", "hardtekk", "zaag",
    "frenchcore", "uptempo", "hardcore", "edm", "electro", "rave", "club",
    "garage", "bassline", "breakbeat", "jungle",
)


@dataclass(frozen=True)
class Mood:
    """One mood: a name, French UI copy, and the bands a title must meet.

    A band of ``None`` means "not required"; a band that is set requires its
    measure, so an unmeasured title can never slip through it.
    """

    id: str
    name: str
    subtitle: str
    energy: tuple[float, float]
    bpm: tuple[float, float] | None = None
    centroid: tuple[float, float] | None = None
    flux: tuple[float, float] | None = None
    styles: tuple[str, ...] = ()

    def fits(self, f) -> bool:
        if not (self.energy[0] <= f.energy <= self.energy[1]):
            return False
        if self.bpm is not None and (
            f.bpm is None or not (self.bpm[0] <= f.bpm <= self.bpm[1])
        ):
            return False
        if self.centroid is not None and (
            f.centroid is None or not (self.centroid[0] <= f.centroid <= self.centroid[1])
        ):
            return False
        if self.flux is not None and (
            f.flux is None or not (self.flux[0] <= f.flux <= self.flux[1])
        ):
            return False
        return True


MOODS = (
    Mood("calme", "Calme", "Souffle lent, sans urgence",
         energy=(0.0, 0.28), bpm=(45.0, 105.0), centroid=(0.35, 1.0)),
    Mood("concentration", "Concentration", "Régulier, sans à-coups",
         energy=(0.24, 0.58), bpm=(90.0, 135.0), centroid=(0.15, 0.60),
         flux=(0.0, 0.45)),
    Mood("bonne-humeur", "Bonne humeur", "Clair et entraînant",
         energy=(0.32, 0.68), centroid=(0.62, 1.0)),
    Mood("energie", "Énergie", "Le tempo qui pousse",
         energy=(0.72, 1.0), bpm=(108.0, 155.0)),
    Mood("soiree", "Soirée", "Fait pour danser",
         energy=(0.72, 1.0), bpm=(118.0, 200.0), styles=DANCE_STYLES),
    Mood("nuit", "Nuit", "Sombre et ralenti",
         energy=(0.0, 0.28), bpm=(45.0, 95.0), centroid=(0.0, 0.35)),
)
#: The ids a client may ask for; anything else is a 404 without any work.
IDS = frozenset(m.id for m in MOODS)
_BY_ID = {m.id: m for m in MOODS}


@dataclass(frozen=True)
class _Features:
    energy: float
    bpm: float | None
    centroid: float | None
    flux: float | None
    style: str | None


@dataclass(frozen=True)
class MoodList:
    """One mood's matching titles for one person's day."""

    mood: Mood
    items: tuple
    opening: object | None

    @property
    def tracks(self) -> int:
        return len(self.items)

    @property
    def ready(self) -> bool:
        return len(self.items) >= READY_MIN

    @property
    def cover_album(self):
        return None if self.opening is None else self.opening.album


class Catalog:
    """The six moods, resolved once against one person's library/day."""

    __slots__ = ("day", "user_id", "moods", "at")

    def __init__(self, day, user_id, moods, at=None):
        self.day = day
        self.user_id = user_id
        self.moods = moods
        self.at = time.monotonic() if at is None else at

    def get(self, mood_id):
        return self.moods.get(mood_id)

    def entries(self):
        """The six definitions in display order, even when one has no match."""
        return [self.moods[m.id] for m in MOODS]

    def mix_ids(self, mood_id, limit=LIST_MAX):
        """The titles of one mood's mix, in play order (row UUIDs).

        The OPENING title is the same one the card's cover came from, so a card
        and the page behind it never disagree; the rest is ``mix.assemble``
        (artist cap 3, artist gap 3, windowed MMR). ``relax=False`` is what
        keeps a prolific artist from filling the list: a mood with too few
        artists simply yields a shorter mix.
        """
        entry = self.moods.get(mood_id)
        if entry is None or not entry.ready or entry.opening is None:
            return []
        try:
            n = max(1, min(int(limit), LIST_MAX))
        except (TypeError, ValueError):
            n = LIST_MAX
        first = entry.opening
        if n == 1:
            return [first.track]
        rest = [it for it in entry.items if it is not first]
        rng = random.Random(  # nosec B311 - a seeded draw so a mix repeats, not a secret
            "%s:%s:%s:list" % (self.user_id or 0, self.day, mood_id)
        )
        # `mix.assemble` writes `_g` on the Items it is given; the catalog is
        # shared by every request in this process, so two people opening the
        # same mood at once must not interleave their draws.
        with _mix_lock:
            out = [first] + mix.assemble(rest, n - 1, rng, fixed=[first], relax=False)
        return [it.track for it in out]


def _json(text):
    try:
        value = json.loads(text) if text else {}
    except (TypeError, ValueError):
        return {}
    return value if isinstance(value, dict) else {}


def _num(value):
    try:
        value = float(value)
    except (TypeError, ValueError):
        return None
    return value if math.isfinite(value) else None


def _fold(text):
    if not text:
        return ""
    text = unicodedata.normalize("NFKD", str(text))
    return "".join(ch for ch in text if not unicodedata.combining(ch)).lower()


def _is_dance(style):
    folded = _fold(style)
    return bool(folded) and any(key in folded for key in DANCE_STYLES)


def _ranks(values):
    """``{key: rank in 0..1}`` (mid-rank for ties), as ``energy._ranks``."""
    if not values:
        return {}
    ordered = sorted(values.values())
    n = len(ordered)
    if n == 1:
        return {k: 0.5 for k in values}
    out = {}
    for k, v in values.items():
        lo = bisect.bisect_left(ordered, v)
        hi = bisect.bisect_right(ordered, v) - 1
        out[k] = ((lo + hi) / 2.0) / (n - 1)
    return out


def _fit(features, mood) -> float:
    """How centred a title is in the bands (1 at the middle, 0 at the edge)."""
    parts = []
    for value, band in (
        (features.energy, mood.energy),
        (features.bpm, mood.bpm),
        (features.centroid, mood.centroid),
        (features.flux, mood.flux),
    ):
        if band is None or value is None:
            continue
        lo, hi = band
        half = (hi - lo) / 2.0
        if half <= 0:
            parts.append(1.0)
            continue
        middle = (lo + hi) / 2.0
        parts.append(max(0.0, 1.0 - abs(value - middle) / half))
    return sum(parts) / len(parts) if parts else 0.5


def _score(features, mood, taste, track_id) -> float:
    """A candidate's weight: band fit × the person's taste, 0 when excluded."""
    score = 0.6 + 0.8 * _fit(features, mood)
    if taste is not None:
        factor = taste.dislikes.factor(track_id)
        if factor <= 0:
            return 0.0
        score *= factor
        skip = taste.history.skip_ratio(track_id)
        if skip:
            score *= 1.0 - 0.6 * skip
        score *= 0.6 + 0.8 * min(1.0, max(0.0, taste.tracks.get(track_id, 0.0)))
    if mood.styles and _is_dance(features.style):
        score *= 1.35
    return max(1e-3, score)


def _trusted_bpm(bpm, confidence):
    value = _num(bpm)
    try:
        conf = float(confidence or 0.0)
    except (TypeError, ValueError):
        conf = 0.0
    return value if value and conf >= energy.BPM_TRUSTED else None


def _query(user):
    query = (
        TrackAnalysis.select(
            TrackAnalysis.track,
            TrackAnalysis.bpm,
            TrackAnalysis.bpm_confidence,
            TrackAnalysis.style,
            TrackAnalysis.data,
            Track.album,
            Track.artist,
        )
        .join(Track, on=(TrackAnalysis.track == Track.id))
        .where(
            (Track.last_modification > 0)
            & Track.unavailable.is_null(True)
        )
    )
    if user is not None and not getattr(user, "admin", False):
        query = query.where(Track.owner.is_null(True) | (Track.owner == user.id))
    return query


def _taste(user):
    if user is None:
        return None
    try:
        return discover.profile(user)
    except Exception:  # a broken taste must never cost somebody their home
        logger.warning("moods: could not read the taste profile", exc_info=True)
        return None


def _opening(items, user_id, day, mood_id):
    """The first title of the mix: one of the best-fitting, drawn per day."""
    pool = sorted(items, key=lambda it: (-it.score, str(it.track)))[:FIRST_POOL]
    rng = random.Random(  # nosec B311 - a seeded draw so a card repeats, not a secret
        "%s:%s:%s:open" % (user_id or 0, day, mood_id)
    )
    return rng.choice(pool) if pool else None


def _build(user, day):
    rows = list(_query(user).tuples())
    energies = energy.energies()
    parsed = []
    centroid_values = {}
    flux_values = {}
    for tid, bpm, conf, style, data, album_id, artist_id in rows:
        value = energies.get(tid)
        if value is None:  # no measured energy: not eligible for any mood
            continue
        payload = _json(data)
        centroid = _num(payload.get("centroid"))
        flux = _num(payload.get("fluxPeak"))
        parsed.append((tid, value, _trusted_bpm(bpm, conf), centroid, flux, style,
                       album_id, artist_id))
        if centroid is not None:
            centroid_values[tid] = centroid
        if flux is not None:
            flux_values[tid] = flux
    centroid_ranks = _ranks(centroid_values)
    flux_ranks = _ranks(flux_values)
    taste = _taste(user)
    collected = {m.id: [] for m in MOODS}
    for tid, value, bpm, _centroid, _flux, style, album_id, artist_id in parsed:
        if taste is not None and taste.history.rejected(tid):
            continue
        features = _Features(
            energy=value,
            bpm=bpm,
            centroid=centroid_ranks.get(tid),
            flux=flux_ranks.get(tid),
            style=style,
        )
        for mood in MOODS:
            if not mood.fits(features):
                continue
            score = _score(features, mood, taste, tid)
            if score <= 0:
                continue
            collected[mood.id].append(
                mix.Item(str(tid), score, artist=artist_id, album=album_id,
                         style=style, track=tid)
            )
    user_id = getattr(user, "id", None)
    entries = {}
    for mood in MOODS:
        items = tuple(collected[mood.id])
        entries[mood.id] = MoodList(
            mood=mood,
            items=items,
            opening=_opening(items, user_id, day, mood.id),
        )
    return Catalog(day, user_id, entries)


_lock = threading.Lock()
#: Serialises the in-place Gumbel draw of one cached catalog's Items.
_mix_lock = threading.Lock()
_cache = {}


def catalog(user=None, day=None) -> Catalog:
    """The person's mood catalog for ``day`` (today by default), cached."""
    day = day or now().date().isoformat()
    user_id = getattr(user, "id", None)
    # The database is part of the key: a process that swaps its database (a
    # test, a reconfiguration) must never read a catalog built from another.
    key = (id(getattr(db, "obj", None)), user_id, day)
    with _lock:
        hit = _cache.get(key)
        if hit is not None and time.monotonic() - hit[0] < TTL:
            return hit[1]
    built = _build(user, day)
    with _lock:
        if len(_cache) >= MAX_CACHED:
            _cache.clear()
        _cache[key] = (time.monotonic(), built)
    return built


def invalidate(user=None):
    """Forget one person's catalogs (tests, or after a library change)."""
    user_id = getattr(user, "id", user)
    with _lock:
        if user_id is None:
            _cache.clear()
            return
        for key in [k for k in _cache if k[1] == user_id]:
            _cache.pop(key, None)


def definition(mood_id):
    """The published definition of a mood, or ``None``."""
    return _BY_ID.get(mood_id)
