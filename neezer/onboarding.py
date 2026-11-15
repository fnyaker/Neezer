# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Pure onboarding logic: artist cards, diversity, similar picks and search ranking.

Nothing here reads a database, the disk or a provider. The same functions run
on the server (the feed) and are pinned by tests without network: a deterministic
order for a seed, sliding caps per genre/region/country/source, provider-order
similar picks, and a search merge that never loses a homonym.
"""

from __future__ import annotations

import dataclasses
import hashlib
import math
import re
import time
import unicodedata
from collections import Counter, OrderedDict, defaultdict
from dataclasses import dataclass

from .deezer import ids

MAX_FEED = 600
PAGE = 24
PAGE_MAX = 48
DIVERSITY = {
    "window": 12,
    "max_same_genre": 4,
    "region_window": 12,
    "region_max": 5,
    "chart_max": 3,
    "country_window": 6,
    "country_max": 1,
    "max_streak": 2,
}
COVERAGE = 8
SIMILAR_COUNT = 4
SIMILAR_MIN = 3
SIMILAR_MAX = 5

COUNTRIES = ("FR", "GB", "US", "DE", "BR", "MX", "JP", "KR", "NG")
COUNTRY_NAMES = {
    "FR": "France",
    "GB": "Royaume-Uni",
    "US": "États-Unis",
    "DE": "Allemagne",
    "BR": "Brésil",
    "MX": "Mexique",
    "JP": "Japon",
    "KR": "Corée du Sud",
    "NG": "Nigeria",
}
REGION = {
    "FR": "Europe", "GB": "Europe", "DE": "Europe", "IT": "Europe", "ES": "Europe",
    "NL": "Europe", "SE": "Europe", "PL": "Europe", "IE": "Europe", "AT": "Europe",
    "CH": "Europe", "BE": "Europe", "PT": "Europe", "NO": "Europe", "DK": "Europe",
    "FI": "Europe", "CZ": "Europe", "GR": "Europe", "RO": "Europe", "HU": "Europe",
    "US": "Amériques", "CA": "Amériques", "MX": "Amériques", "BR": "Amériques",
    "AR": "Amériques", "CL": "Amériques", "CO": "Amériques", "PE": "Amériques",
    "JP": "Asie", "KR": "Asie", "IN": "Asie", "ID": "Asie", "PH": "Asie",
    "TH": "Asie", "SG": "Asie", "MY": "Asie", "CN": "Asie", "TW": "Asie",
    "NG": "Afrique", "ZA": "Afrique", "CI": "Afrique", "SN": "Afrique",
    "EG": "Afrique", "MA": "Afrique", "KE": "Afrique", "TN": "Afrique",
    "DZ": "Afrique",
    "AU": "Océanie", "NZ": "Océanie",
}
REGION_BY_GENRE = {
    "chanson francaise": "Europe",
    "musique africaine": "Afrique",
    "musique arabe": "Afrique",
    "musique asiatique": "Asie",
    "musique indienne": "Asie",
    "musique bresilienne": "Amériques",
    "latino": "Amériques",
}
_PLACEHOLDERS = {
    "", "[unknown]", "unknown", "unknown artist", "various artists",
    "various", "artiste inconnu", "compilation",
}
_CONTROL = re.compile(r"[\x00-\x1f\x7f-\x9f]")
_YT_REF = re.compile(r"\A[A-Za-z0-9_-]{10,128}\Z")
_NUMERIC = re.compile(r"\A[0-9]{1,20}\Z")
_UUID = re.compile(
    r"\A[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-"
    r"[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\Z"
)


def fold_name(value) -> str:
    text = unicodedata.normalize("NFKD", str(value or ""))
    text = "".join(c for c in text if not unicodedata.combining(c))
    return " ".join(text.lower().split())


def clean_name(value) -> str | None:
    text = _CONTROL.sub(" ", str(value or ""))
    text = " ".join(text.split()).strip()[:256]
    if text.lower() in _PLACEHOLDERS:
        return None
    if text.endswith(" - Topic"):
        text = text[: -len(" - Topic")].strip()
    return text or None


def valid_deezer_ref(value) -> bool:
    return bool(_NUMERIC.fullmatch(str(value or "")))


def valid_uuid(value) -> bool:
    return bool(_UUID.fullmatch(str(value or "")))


def valid_youtube_ref(value) -> bool:
    return bool(_YT_REF.fullmatch(str(value or "")))


@dataclass(frozen=True)
class Card:
    provider: str
    ref: str
    name: str
    picture: str | None = None
    picture_small: str | None = None
    genres: tuple[str, ...] = ()
    countries: tuple[str, ...] = ()
    rank: int = 1000
    source: str = "chart"
    sources: tuple[str, ...] = ()
    fans: int = 0
    subscribers: int = 0
    parent: str | None = None
    hint: str | None = None

    @property
    def key(self) -> str:
        return f"{self.provider}:{self.ref}"

    @property
    def entity_id(self) -> str:
        if self.provider == "deezer":
            return str(ids.artist_uuid(self.ref))
        if self.provider == "youtube":
            return str(ids.youtube_artist_uuid(self.ref))
        return self.ref

    @property
    def api_id(self) -> str:
        return self.ref if self.provider == "deezer" else self.entity_id

    @property
    def openable(self) -> bool:
        return self.provider != "youtube"

    def picture_url(self, small=False):
        if self.provider == "youtube":
            suffix = "?s=60" if small else ""
            return f"/api/onboarding/picture/{self.entity_id}{suffix}"
        return self.picture_small if small else self.picture

    def as_item(self, liked=False, hint=None, via=None, parent=None) -> dict:
        return {
            "deezer_id": self.api_id,
            "id": self.api_id,
            "name": self.name,
            "picture": self.picture_url(),
            "picture_small": self.picture_url(small=True),
            "genres": list(self.genres),
            "country": self.countries[0] if self.countries else None,
            "liked": bool(liked),
            "provider": self.provider,
            "openable": self.openable,
            "hint": hint if hint is not None else self.hint,
            "via": via or self.source,
            "parent": parent if parent is not None else self.parent,
        }

    def as_dict(self) -> dict:
        return {
            "provider": self.provider,
            "ref": self.ref,
            "name": self.name,
            "picture": self.picture,
            "picture_small": self.picture_small,
            "genres": list(self.genres),
            "countries": list(self.countries),
            "rank": self.rank,
            "source": self.source,
            "sources": list(self.sources),
            "fans": self.fans,
            "subscribers": self.subscribers,
        }

    @classmethod
    def from_dict(cls, data: dict) -> "Card | None":
        provider = str(data.get("provider") or "")
        ref = str(data.get("ref") or "")
        name = clean_name(data.get("name"))
        if provider not in ("deezer", "youtube", "local") or not name:
            return None
        if provider == "deezer" and not valid_deezer_ref(ref):
            return None
        if provider == "youtube" and not valid_youtube_ref(ref):
            return None
        if provider == "local" and not valid_uuid(ref):
            return None
        return cls(
            provider=provider,
            ref=ref,
            name=name,
            picture=(str(data["picture"])[:512] if data.get("picture") else None),
            picture_small=(
                str(data["picture_small"])[:512] if data.get("picture_small") else None
            ),
            genres=tuple(str(g)[:80] for g in (data.get("genres") or ()) if str(g).strip()),
            countries=tuple(
                str(c).upper()[:2] for c in (data.get("countries") or ()) if str(c).strip()
            ),
            rank=max(1, int(data.get("rank") or 1000)),
            source=str(data.get("source") or "chart")[:24],
            sources=tuple(str(s)[:24] for s in (data.get("sources") or ()) if str(s).strip()),
            fans=max(0, int(data.get("fans") or 0)),
            subscribers=max(0, int(data.get("subscribers") or 0)),
        )


def seed_for(user_id, build_id) -> str:
    value = f"{user_id}:{build_id or '0'}:onboarding".encode("utf-8")
    return hashlib.blake2b(value, digest_size=8).hexdigest()


def _jitter(seed: str, key: str) -> float:
    digest = hashlib.blake2b(
        f"{seed}:{key}".encode("utf-8"), digest_size=8
    ).digest()
    return int.from_bytes(digest, "big") / float(1 << 64)


def _folded_genres(card: Card) -> tuple[str, ...]:
    return tuple(g for g in (fold_name(x) for x in card.genres) if g)


def _country(card: Card) -> str | None:
    return card.countries[0].upper() if card.countries else None


def _country_region(card: Card) -> str:
    country = _country(card)
    if country:
        return REGION.get(country, "?")
    return REGION_BY_GENRE.get(_primary_genre(card), "")


def _primary_genre(card: Card) -> str:
    genres = _folded_genres(card)
    return genres[0] if genres else ""


def _primary_bucket(card: Card, genre_counts: Counter) -> str:
    if card.provider == "local":
        return "library"
    if card.provider == "youtube":
        country = _country(card)
        return f"yt:{country}" if country else "yt:global"
    if "chart" in card.sources or "youtube-chart" in card.sources:
        return "chart"
    country = _country(card)
    if country and "country" in card.sources:
        return f"country:{country}"
    genres = _folded_genres(card)
    if genres:
        return "genre:" + min(genres, key=lambda g: (genre_counts.get(g, 0), g))
    if country:
        return f"country:{country}"
    return "chart"


def _region(card: Card, bucket: str) -> str:
    if bucket.startswith("country:") or bucket.startswith("yt:"):
        return _country_region(card)
    if bucket.startswith("genre:"):
        return REGION_BY_GENRE.get(bucket.split(":", 1)[1], "") or "?"
    if bucket == "library":
        return "library"
    return "?"


def _score(card: Card, seed: str) -> float:
    rank = 1.0 / (1.0 + max(0, card.rank))
    novelty = 1.0 if card.source in ("genre", "country", "youtube-chart") else (
        0.2 if card.source == "chart" else 0.5
    )
    audience = card.fans or card.subscribers
    audience_score = 0.0
    if audience:
        audience_score = min(1.0, math.log1p(audience) / 15.0)
    return (
        0.45 * rank
        + 0.25 * novelty
        + 0.20 * _jitter(seed, card.key)
        + 0.10 * audience_score
    )


def _violates(out, bucket, region, window, caps) -> bool:
    if out and len(out) >= caps["max_streak"]:
        tail = out[-caps["max_streak"]:]
        if all(item[0] == bucket for item in tail):
            return True
    recent = out[-caps["window"]:]
    if bucket.startswith("genre:") and sum(1 for item in recent if item[0] == bucket) >= caps["max_same_genre"]:
        return True
    if bucket == "chart" and sum(1 for item in recent if item[0] == "chart") >= caps["chart_max"]:
        return True
    if bucket.startswith(("country:", "yt:")) and region and region != "?":
        country = bucket.split(":", 1)[1]
        countries = [
            item[1] for item in out[-caps["country_window"]:]
            if item[0].split(":", 1)[0] in ("country", "yt")
        ]
        if countries.count(country) >= caps["country_max"]:
            return True
    if region and region != "?":
        regions = [item[2] for item in recent]
        if regions.count(region) >= caps["region_max"]:
            return True
    return False


def diversify(cards, seed: str, limit=None) -> list[Card]:
    """Deterministic order of the pool: genre, nine countries, regions, chart."""
    caps = dict(DIVERSITY)
    unique = []
    seen = set()
    for card in cards:
        if card.key in seen or not card.name:
            continue
        seen.add(card.key)
        unique.append(card)
    if not unique:
        return []
    genre_counts = Counter(g for card in unique for g in _folded_genres(card))
    buckets = defaultdict(list)
    regions = {}
    for card in unique:
        bucket = _primary_bucket(card, genre_counts)
        buckets[bucket].append(card)
        regions[card.key] = _region(card, bucket)
    for bucket in buckets:
        buckets[bucket].sort(key=lambda c: (-_score(c, seed), c.key))
    files = {b: list(v) for b, v in buckets.items() if v}
    if not files:
        return []
    weights = {b: max(1.0, len(v) ** 0.5) for b, v in files.items()}
    total = sum(weights.values())
    current = defaultdict(float)
    covered = set()
    out = []
    target = min(limit, MAX_FEED) if limit else MAX_FEED
    while files and len(out) < target:
        state = [
            (
                item[1],
                item[1].split(":", 1)[1]
                if item[1].startswith(("country:", "yt:"))
                else "",
                item[2],
            )
            for item in out
        ]
        eligible = [
            b for b in files
            if not _violates(
                state,
                b,
                regions[files[b][0].key],
                caps["window"],
                caps,
            )
        ]
        if not eligible:
            eligible = [
                b for b in files
                if not _violates(
                    state,
                    b,
                    regions[files[b][0].key],
                    caps["window"],
                    dict(caps, max_streak=0),
                )
            ]
        if not eligible:
            eligible = list(files)
        pool = eligible
        fresh = [b for b in eligible if b not in covered]
        if fresh and len(covered) < COVERAGE:
            used_regions = {item[2] for item in out}
            distinct = [
                b for b in fresh
                if regions[files[b][0].key] not in used_regions
            ]
            pool = distinct or fresh
            bucket = max(
                pool,
                key=lambda b: (weights[b], _jitter(seed, "bucket:" + b)),
            )
        else:
            for b in eligible:
                current[b] += weights[b]
            bucket = max(
                eligible,
                key=lambda b: (current[b], _jitter(seed, "bucket:" + b)),
            )
            current[bucket] -= total
        card = files[bucket].pop(0)
        out.append((card, bucket, regions[card.key]))
        covered.add(bucket)
        if not files[bucket]:
            del files[bucket]
    return [item[0] for item in out]


def pick_similar(
    candidates,
    exclude_refs=(),
    exclude_names=(),
    count=SIMILAR_COUNT,
) -> list[Card]:
    """Provider order, filtered; never fills a hole with an unrelated artist."""
    count = max(SIMILAR_MIN, min(SIMILAR_MAX, int(count or SIMILAR_COUNT)))
    excluded = {str(value) for value in exclude_refs}
    names = {fold_name(value) for value in exclude_names if fold_name(value)}
    per_genre = Counter()
    out = []
    for card in candidates:
        if card.api_id in excluded or card.ref in excluded or card.entity_id in excluded:
            continue
        folded = fold_name(card.name)
        if not folded or folded in names:
            continue
        genre = _primary_genre(card)
        if genre and per_genre[genre] >= 3:
            continue
        out.append(card)
        names.add(folded)
        if genre:
            per_genre[genre] += 1
        if len(out) >= count:
            break
    return out


def _match_tier(name: str, query: str) -> int:
    folded = fold_name(name)
    if folded == query:
        return 0
    if folded.startswith(query):
        return 1
    if any(word.startswith(query) for word in folded.split()):
        return 2
    if query in folded:
        return 3
    return 4


def _fan_hint(fans: int) -> str:
    if fans >= 1_000_000:
        return f"{fans / 1_000_000:.1f}".replace(".", ",") + " M fans"
    if fans >= 1_000:
        return f"{fans // 1_000} K fans"
    return f"{fans} fans"


def _hint(card: Card) -> str:
    country = _country(card)
    if country:
        return COUNTRY_NAMES.get(country, country)
    if card.genres:
        return card.genres[0]
    if card.fans:
        return _fan_hint(card.fans)
    return {"deezer": "Deezer", "youtube": "YouTube", "local": "Bibliothèque"}.get(
        card.provider, card.provider
    )


def rank_search(local, remote, query) -> list[Card]:
    """Local cards first at equal relevance; never merge two ids by name."""
    folded = fold_name(query)
    merged = OrderedDict()
    for card in list(local) + list(remote):
        if card.key not in merged:
            merged[card.key] = card
    local_keys = {card.key for card in local}
    ranked = sorted(
        merged.values(),
        key=lambda c: (
            _match_tier(c.name, folded),
            0 if c.key in local_keys else 1,
            -(c.fans or c.subscribers or 0),
            fold_name(c.name),
            c.api_id,
        ),
    )
    names = Counter(fold_name(card.name) for card in ranked)
    out = []
    for card in ranked:
        if names[fold_name(card.name)] > 1:
            out.append(dataclasses.replace(card, hint=_hint(card)))
        else:
            out.append(card)
    return out


def page_hints(cards) -> dict:
    names = Counter(fold_name(card.name) for card in cards)
    return {
        card.key: _hint(card) if names[fold_name(card.name)] > 1 else None
        for card in cards
    }


def read_after(value) -> int | None:
    if value in (None, ""):
        return 0
    try:
        position = int(value)
    except (TypeError, ValueError):
        return None
    if 0 <= position <= MAX_FEED:
        return position
    return None


def next_after(position: int, total: int) -> str | None:
    if position >= min(total, MAX_FEED):
        return None
    return str(min(position, MAX_FEED))


class SearchCache:
    def __init__(self, size=256):
        self.size = max(1, int(size))
        self._entries = OrderedDict()

    def get(self, key):
        entry = self._entries.get(key)
        if entry is None:
            return None
        if entry[0] <= time.monotonic():
            self._entries.pop(key, None)
            return None
        self._entries.move_to_end(key)
        return entry[1]

    def put(self, key, value, ttl):
        self._entries[key] = (time.monotonic() + max(0.0, ttl), value)
        self._entries.move_to_end(key)
        while len(self._entries) > self.size:
            self._entries.popitem(last=False)

    def clear(self):
        self._entries.clear()


search_cache = SearchCache()


def reset_caches() -> None:
    search_cache.clear()


def warm_feed(app, user=None) -> None:
    import threading

    from .deezer import onboarding_pool

    threading.Thread(
        target=onboarding_pool.warm_feed,
        args=(app, user),
        name="onboarding-warm",
        daemon=True,
    ).start()


def start(app) -> None:
    from .deezer import onboarding_pool

    onboarding_pool.start(app)
