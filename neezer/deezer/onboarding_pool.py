# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""The onboarding artist pool: public Deezer charts, the local library, YouTube.

The pool is built off the request path, one worker at a time, and cached on disk
for a day. A build never replaces a good pool with a failed or thin one, and a
request only ever reads the file or the database. The order is the owner's:
public Deezer charts and the nine country Top playlists, then the local library
when it has a page, then YouTube Music only when Deezer failed and the library
is short.
"""

from __future__ import annotations

import hashlib
import json
import logging
import os
import re
import threading
import time
from collections import OrderedDict, deque
from dataclasses import replace
from datetime import datetime, timedelta
from urllib.parse import urlsplit

import requests
from peewee import fn

from .. import access
from ..db import Artist, ArtistLink, Meta, ProviderAccount, Track, now
from ..deezer import workload
from ..deezer import youtube as yt
from ..onboarding import (
    COUNTRIES,
    Card,
    clean_name,
    fold_name,
    page_hints,
    pick_similar,
    search_cache,
    valid_deezer_ref,
    valid_uuid,
    valid_youtube_ref,
)

logger = logging.getLogger(__name__)

POOL_VERSION = 1
MIN_POOL = 80
MIN_SOURCES = 3
POOL_TTL = 24 * 3600
BUILD_BUDGET = 45.0
BOOT_DELAY = 60.0
CALL_PAUSE = 0.25
RELATED_TTL = 24 * 3600
RELATED_MAX = 20
RELATED_QUEUE = 50
ARTIST_TTL = 3600
ARTIST_CACHE_MAX = 2000
PICTURE_MAX = 4 * 1024 * 1024
PICTURE_HOSTS = {"yt3.googleusercontent.com", "lh3.googleusercontent.com"}
YOUTUBE_COUNTRIES = ("ZZ",) + COUNTRIES
MAX_CHART_ARTISTS = 100
MAX_COUNTRY_ARTISTS = 40
MAX_GENRE_ARTISTS = 20
MAX_GENRE_CHARTS = 10

COUNTRY_PLAYLISTS = {
    "FR": "1109890291",
    "GB": "1111142221",
    "US": "1313621735",
    "DE": "1111143121",
    "BR": "1111141961",
    "MX": "1111142361",
    "JP": "1362508955",
    "KR": "1362510315",
    "NG": "1362516565",
}
PREFERRED_GENRES = (
    "Pop", "Rap/Hip Hop", "Rock", "Electro", "R&B", "Jazz", "Classique",
    "Reggae", "Metal", "Folk",
)
NON_MUSIC_GENRES = {
    "all", "livres audio", "audiobooks", "films/jeux video", "films/games",
    "jeunesse", "children's music", "podcasts",
}

_POOL_FILES = {"deezer": "artists.json", "youtube": "artists-youtube.json"}
_lock = threading.RLock()
_loaded = {}
_building = {"deezer": False, "youtube": False}
_progress = {"deezer": (0, 0), "youtube": (0, 0)}
_errors = {"deezer": None, "youtube": None}
_attempted = {"deezer": False, "youtube": False}
_client_override = None
_boot_started = False

_related_lock = threading.Lock()
_related_queue = deque()
_related_queued = set()
_related_status = {}
_related_running = False

_picture_lock = threading.Lock()
_picture_failures = {}


class PoolError(Exception):
    def __init__(self, code, cards=(), sources_ok=None):
        super().__init__(code)
        self.code = code
        self.cards = list(cards)
        self.sources_ok = sources_ok


class ArtistCache:
    def __init__(self, ttl=ARTIST_TTL, size=ARTIST_CACHE_MAX):
        self.ttl = ttl
        self.size = size
        self._items = OrderedDict()
        self._by_entity = {}
        self._lock = threading.Lock()

    def remember(self, cards) -> None:
        deadline = time.monotonic() + self.ttl
        with self._lock:
            for card in cards:
                if not card.name:
                    continue
                key = (card.provider, card.ref)
                self._items[key] = (deadline, card)
                self._items.move_to_end(key)
                self._by_entity[card.entity_id] = key
            while len(self._items) > self.size:
                _key, _value = self._items.popitem(last=False)
                self._by_entity = {
                    entity: key for entity, key in self._by_entity.items()
                    if key in self._items
                }

    def get(self, provider, ref):
        key = (provider, str(ref))
        with self._lock:
            entry = self._items.get(key)
            if entry is None:
                return None
            if entry[0] <= time.monotonic():
                self._items.pop(key, None)
                self._by_entity.pop(entry[1].entity_id, None)
                return None
            self._items.move_to_end(key)
            return entry[1]

    def by_entity(self, entity_id):
        entity_id = str(entity_id)
        with self._lock:
            key = self._by_entity.get(entity_id)
            if key is None:
                return None
            entry = self._items.get(key)
            if entry is None or entry[0] <= time.monotonic():
                self._items.pop(key, None)
                self._by_entity.pop(entity_id, None)
                return None
            return entry[1]

    def clear(self):
        with self._lock:
            self._items.clear()
            self._by_entity.clear()


artist_cache = ArtistCache()


def set_public_client(client) -> None:
    global _client_override
    _client_override = client


def deezer_enabled(app=None) -> bool:
    try:
        if app is None:
            from flask import current_app

            app = current_app
        config = app.config.get("DEEZER") or {}
    except RuntimeError:
        return True
    return bool(config.get("enabled"))


def public_client():
    if _client_override is not None:
        return _client_override
    try:
        from flask import current_app

        if current_app.testing:
            return None
        if not deezer_enabled(current_app):
            return None
    except RuntimeError:
        return None
    from ..deezer.accounts import public_api

    return public_api()


def reset_state() -> None:
    global _client_override, _related_running, _boot_started
    with _lock:
        _loaded.clear()
        for kind in _building:
            _building[kind] = False
            _progress[kind] = (0, 0)
            _errors[kind] = None
            _attempted[kind] = False
    with _related_lock:
        _related_queue.clear()
        _related_queued.clear()
        _related_status.clear()
        _related_running = False
    with _picture_lock:
        _picture_failures.clear()
    artist_cache.clear()
    search_cache.clear()
    _client_override = None
    _boot_started = False


def _meta(key, default=None):
    try:
        row = Meta.get_or_none(Meta.key == key)
    except Exception:
        return default
    return row.value if row is not None else default


def _root(app) -> str:
    return os.path.join(app.config["WEBAPP"]["cache_dir"], "onboarding")


def _pool_path(app, kind) -> str:
    return os.path.join(_root(app), _POOL_FILES[kind])


def _build_id(moment) -> str:
    return moment.strftime("%Y%m%dT%H%M")


def _write_json(path, data) -> None:
    directory = os.path.dirname(path)
    os.makedirs(directory, exist_ok=True)
    temporary = path + ".tmp"
    with open(temporary, "w", encoding="utf-8") as handle:
        json.dump(data, handle, ensure_ascii=False, separators=(",", ":"))
        handle.flush()
        try:
            os.fsync(handle.fileno())
        except OSError:
            pass
    os.replace(temporary, path)


def _read_json(path):
    try:
        with open(path, "r", encoding="utf-8") as handle:
            return json.load(handle)
    except (OSError, ValueError):
        return None


def _inspect_snapshot(data):
    if not isinstance(data, dict) or data.get("v") != POOL_VERSION:
        return None
    cards = []
    for raw in data.get("artists") or ():
        card = Card.from_dict(raw)
        if card is not None:
            cards.append(card)
    if not cards:
        return None
    return {
        "v": POOL_VERSION,
        "build": str(data.get("build") or ""),
        "built_at": str(data.get("built_at") or ""),
        "sources": data.get("sources") or {},
        "partial": bool(data.get("partial")),
        "error": data.get("error"),
        "cards": cards,
        "artists": data.get("artists") or [],
    }


def get_pool(app, kind="deezer"):
    path = _pool_path(app, kind)
    try:
        mtime = os.stat(path).st_mtime_ns
    except OSError:
        return None
    with _lock:
        cached = _loaded.get(kind)
        if cached is not None and cached[0] == mtime:
            return cached[1]
    data = _inspect_snapshot(_read_json(path))
    with _lock:
        _loaded[kind] = (mtime, data)
    return data


def _fresh(snapshot) -> bool:
    try:
        built = datetime.fromisoformat(snapshot["built_at"])
    except (TypeError, ValueError):
        return False
    return now() - built <= timedelta(seconds=POOL_TTL)


def _snapshot_cards(snapshot):
    return list(snapshot.get("cards") or ()) if snapshot else []


def state_of(app, kind="deezer") -> dict:
    snapshot = get_pool(app, kind)
    cards = _snapshot_cards(snapshot)
    with _lock:
        building = _building.get(kind, False)
        progress = _progress.get(kind) or (0, 0)
        error = _errors.get(kind)
    if error is None and snapshot is not None:
        error = snapshot.get("error")
    return {
        "ready": bool(cards) and (snapshot is not None),
        "building": building,
        "stale": bool(cards) and not _fresh(snapshot),
        "source": kind,
        "build": snapshot.get("build") if snapshot else None,
        "kept": len(cards),
        "progress": {"done": progress[0], "total": progress[1]} if building else None,
        "sources": snapshot.get("sources") if snapshot else {},
        "partial": bool(snapshot and snapshot.get("partial")),
        "error": error,
    }


def youtube_status(user=None) -> dict:
    installed = yt.installed()
    enabled = bool(installed and _meta("yt.enabled", "1") != "0")
    held, retry_in, reason = yt.held()
    denied = bool(user is not None and enabled and not access.can(user, "youtube", "search"))
    error = None
    if not installed:
        error = "absent"
    elif not enabled:
        error = "disabled"
    elif held:
        error = "held"
    elif denied:
        error = "denied"
    return {
        "installed": installed,
        "enabled": enabled,
        "held": held,
        "retry_in": retry_in,
        "reason": reason or None,
        "denied": denied,
        "error": error,
    }


def _youtube_usable(user=None) -> bool:
    status = youtube_status(user)
    return bool(status["enabled"] and not status["held"] and not status["denied"])


def parse_count(value):
    text = str(value or "").strip().replace("\xa0", "").replace(" ", "")
    match = re.match(r"\A([0-9]+(?:[.,][0-9]+)?)\s*([KMB])?\Z", text, re.I)
    if not match:
        return 0
    number = float(match.group(1).replace(",", "."))
    factor = {"K": 1_000, "M": 1_000_000, "B": 1_000_000_000}.get(
        (match.group(2) or "").upper(), 1
    )
    return max(0, int(number * factor))


def _is_music_genre(row) -> bool:
    if not row or not row.get("id"):
        return False
    return fold_name(row.get("name")) not in NON_MUSIC_GENRES


def _artist_of(payload):
    if not isinstance(payload, dict):
        return None
    artist = payload.get("artist")
    if isinstance(artist, dict) and artist.get("id"):
        return artist
    artists = payload.get("artists")
    if isinstance(artists, list):
        for item in artists:
            if isinstance(item, dict) and item.get("id"):
                return item
    contributors = payload.get("contributors")
    if isinstance(contributors, list):
        for item in contributors:
            if isinstance(item, dict) and item.get("id"):
                return item
    if payload.get("id") and payload.get("name"):
        return payload
    return None


def _merge_deezer(merged, raw, source, country=None, genre=None, rank=1):
    if not isinstance(raw, dict):
        return
    ref = str(raw.get("id") or "")
    if not valid_deezer_ref(ref):
        return
    name = clean_name(raw.get("name") or raw.get("title"))
    if not name:
        return
    entry = merged.get(ref)
    if entry is None:
        entry = merged[ref] = {
            "ref": ref,
            "name": name,
            "picture": raw.get("picture_medium") or raw.get("picture_big") or raw.get("picture"),
            "picture_small": raw.get("picture_small") or raw.get("picture_medium"),
            "genres": set(),
            "countries": {},
            "sources": set(),
            "rank": max(1, int(rank or 1)),
            "source": source,
        }
    entry["sources"].add(source)
    if source == "chart" or (source == "country" and entry["source"] == "genre"):
        entry["source"] = "chart" if source == "chart" else "country"
    entry["rank"] = min(entry["rank"], max(1, int(rank or 1)))
    if genre:
        entry["genres"].add(genre)
    if country:
        entry["countries"][country] = min(
            entry["countries"].get(country, rank), max(1, int(rank or 1))
        )
    if not entry["picture"]:
        entry["picture"] = raw.get("picture_medium") or raw.get("picture_big")
    if not entry["picture_small"]:
        entry["picture_small"] = raw.get("picture_small")


def _deezer_snapshot(merged, sources_seen, error, started):
    cards = []
    for entry in merged.values():
        countries = tuple(
            cc for cc, _rank in sorted(entry["countries"].items(), key=lambda item: (item[1], item[0]))
        )
        cards.append(
            Card(
                provider="deezer",
                ref=entry["ref"],
                name=entry["name"],
                picture=(entry["picture"] or None),
                picture_small=(entry["picture_small"] or entry["picture"] or None),
                genres=tuple(sorted(entry["genres"])),
                countries=countries,
                rank=entry["rank"],
                source=entry["source"],
                sources=tuple(sorted(entry["sources"])),
            )
        )
    cards.sort(key=lambda card: (card.rank, card.name))
    return {
        "v": POOL_VERSION,
        "build": _build_id(started),
        "built_at": started.isoformat(),
        "sources": sources_seen,
        "artists": [card.as_dict() for card in cards],
        "partial": bool(error and cards),
        "error": error,
        "cards": cards,
    }


def build_deezer(client, budget=BUILD_BUDGET, progress=None, meta=None) -> dict:
    started = now()
    deadline = time.monotonic() + max(1.0, float(budget))
    meta = meta or _meta
    sources_seen = {}
    merged = {}
    error = None
    done = 0
    total = 2
    genres = []

    def call(method, *args):
        nonlocal done
        workload.quiet_wait(1.0)
        if time.monotonic() > deadline:
            raise PoolError("timeout")
        time.sleep(CALL_PAUSE)
        fn_call = getattr(client, method)
        try:
            payload = fn_call(*args)
        except Exception as exc:
            from deezerpy.errors import is_transport_failure

            if is_transport_failure(exc):
                raise PoolError("deezer-unreachable")
            raise PoolError("error")
        done += 1
        if progress is not None:
            progress(done, total)
        return payload

    if client is None:
        return _deezer_snapshot(merged, sources_seen, "absent", started)

    try:
        raw_genres = call("get_genres", 0, 100)
        genre_rows = [
            row for row in (raw_genres.get("data") or []) if _is_music_genre(row)
        ]
        by_name = {fold_name(row.get("name")): row for row in genre_rows}
        chosen = []
        chosen_ids = set()
        for name in PREFERRED_GENRES:
            row = by_name.get(fold_name(name))
            if row is not None and row["id"] not in chosen_ids:
                chosen.append(row)
                chosen_ids.add(row["id"])
        for row in genre_rows:
            if len(chosen) >= MAX_GENRE_CHARTS:
                break
            if row["id"] not in chosen_ids:
                chosen.append(row)
                chosen_ids.add(row["id"])
        genres = chosen[:MAX_GENRE_CHARTS]
        sources_seen["genres"] = {"ok": True, "n": len(genres)}
        total = 2 + len(COUNTRIES) + len(genres)

        chart = call("get_chart_artists", 0, 0, MAX_CHART_ARTISTS)
        rows = chart.get("data") if isinstance(chart, dict) else chart
        for index, raw in enumerate(rows or []):
            _merge_deezer(merged, raw, "chart", rank=index + 1)
        sources_seen["chart"] = {"ok": True, "n": len(rows or [])}

        for cc in COUNTRIES:
            playlist_id = str(meta(f"onb.ctry.{cc}") or COUNTRY_PLAYLISTS[cc])
            try:
                payload = call("get_playlist_tracks", playlist_id, 0, 50)
            except PoolError as exc:
                sources_seen[f"country:{cc}"] = {"ok": False, "error": exc.code}
                if exc.code in ("deezer-unreachable", "timeout", "quota"):
                    raise
                continue
            rows = payload.get("data") if isinstance(payload, dict) else payload
            count = 0
            for index, track in enumerate(rows or []):
                artist = _artist_of(track)
                if artist is None:
                    continue
                _merge_deezer(merged, artist, "country", country=cc, rank=index + 1)
                count += 1
            sources_seen[f"country:{cc}"] = {"ok": bool(count), "n": count}
            if not count:
                sources_seen[f"country:{cc}"]["error"] = "empty"

        for genre in genres:
            name = str(genre.get("name") or "")
            try:
                payload = call("get_editorial_charts", genre["id"])
            except PoolError as exc:
                sources_seen[f"genre:{name}"] = {"ok": False, "error": exc.code}
                if exc.code in ("deezer-unreachable", "timeout", "quota"):
                    raise
                continue
            count = 0
            for key in ("albums", "tracks"):
                rows = (payload or {}).get(key) or ()
                for index, item in enumerate(rows):
                    artist = _artist_of(item)
                    if artist is None:
                        continue
                    _merge_deezer(merged, artist, "genre", genre=name, rank=index + 1)
                    count += 1
            sources_seen[f"genre:{name}"] = {"ok": bool(count), "n": count}
            if not count:
                sources_seen[f"genre:{name}"]["error"] = "empty"
    except PoolError as exc:
        error = exc.code
    except Exception:
        logger.warning("onboarding: Deezer pool build failed", exc_info=True)
        error = "error"
    if not merged and error is None:
        error = "empty"
    return _deezer_snapshot(merged, sources_seen, error, started)


def _yt_picture(url, size):
    if not isinstance(url, str) or not url:
        return None
    parts = urlsplit(url)
    if parts.scheme != "https" or parts.hostname not in PICTURE_HOSTS:
        return None
    return re.sub(r"=w\d+-h\d+", f"=w{size}-h{size}", url, count=1)


def _best_thumb(thumbs):
    if not isinstance(thumbs, list):
        return None
    best = None
    for thumb in thumbs:
        if not isinstance(thumb, dict):
            continue
        try:
            width = int(thumb.get("width") or 0)
        except (TypeError, ValueError):
            width = 0
        if best is None or width > best[0]:
            best = (width, thumb.get("url"))
    return best[1] if best else None


def _yt_card(raw, cc, index):
    ref = str(raw.get("browseId") or "")
    if ref.startswith("MPLA"):
        ref = ref[4:]
    if not valid_youtube_ref(ref):
        return None
    name = clean_name(raw.get("title"))
    if not name:
        return None
    try:
        rank = max(1, int(raw.get("rank")))
    except (TypeError, ValueError):
        rank = index + 1
    picture = _best_thumb(raw.get("thumbnails"))
    return Card(
        provider="youtube",
        ref=ref,
        name=name,
        picture=_yt_picture(picture, 250),
        picture_small=_yt_picture(picture, 60),
        countries=(cc,) if cc in COUNTRIES else (),
        rank=rank,
        source="youtube-chart",
        sources=("youtube-chart",),
        subscribers=parse_count(raw.get("subscribers")),
    )


def _merge_yt(merged, raw, cc, index):
    card = _yt_card(raw, cc, index)
    if card is None:
        return
    entry = merged.get(card.ref)
    if entry is None:
        merged[card.ref] = card
        return
    countries = entry.countries
    if card.countries and card.countries[0] not in countries:
        countries = countries + card.countries
    if card.rank < entry.rank:
        merged[card.ref] = Card(
            provider="youtube",
            ref=entry.ref,
            name=entry.name,
            picture=entry.picture or card.picture,
            picture_small=entry.picture_small or card.picture_small,
            countries=countries,
            rank=card.rank,
            source=entry.source,
            sources=entry.sources,
            subscribers=max(entry.subscribers, card.subscribers),
        )
    elif countries != entry.countries or (not entry.picture and card.picture):
        merged[card.ref] = Card(
            provider="youtube",
            ref=entry.ref,
            name=entry.name,
            picture=entry.picture or card.picture,
            picture_small=entry.picture_small or card.picture_small,
            countries=countries,
            rank=entry.rank,
            source=entry.source,
            sources=entry.sources,
            subscribers=max(entry.subscribers, card.subscribers),
        )


def _yt_snapshot(merged, sources_seen, error, started):
    cards = sorted(merged.values(), key=lambda card: (card.rank, card.name))
    return {
        "v": POOL_VERSION,
        "build": _build_id(started),
        "built_at": started.isoformat(),
        "sources": sources_seen,
        "artists": [card.as_dict() for card in cards],
        "partial": bool(error and cards),
        "error": error,
        "cards": cards,
        "countries": list(YOUTUBE_COUNTRIES),
    }


def build_youtube(client=None, budget=BUILD_BUDGET, progress=None) -> dict:
    started = now()
    deadline = time.monotonic() + max(1.0, float(budget))
    sources_seen = {}
    merged = {}
    error = None
    consecutive = 0
    total = len(YOUTUBE_COUNTRIES)
    if not yt.installed():
        return _yt_snapshot(merged, sources_seen, "absent", started)
    if _meta("yt.enabled", "1") == "0":
        return _yt_snapshot(merged, sources_seen, "disabled", started)
    held, _retry, _reason = yt.held()
    if held:
        return _yt_snapshot(merged, sources_seen, "held", started)
    if client is None:
        try:
            client = yt._client()
        except Exception:
            return _yt_snapshot(merged, sources_seen, "error", started)

    for index, cc in enumerate(YOUTUBE_COUNTRIES):
        try:
            yt._check_held()
            workload.quiet_wait(1.0)
            if time.monotonic() > deadline:
                raise PoolError("timeout")
            payload = client.get_charts(cc)
        except yt.YoutubeHeld:
            error = "held"
            sources_seen[f"chart:{cc}"] = {"ok": False, "error": "held"}
            break
        except PoolError as exc:
            error = exc.code
            sources_seen[f"chart:{cc}"] = {"ok": False, "error": exc.code}
            break
        except Exception as exc:
            yt._hold_for(exc)
            consecutive += 1
            code = "held" if yt.held()[0] else "error"
            error = code
            sources_seen[f"chart:{cc}"] = {"ok": False, "error": code}
            if consecutive >= 2 or code == "held":
                break
            continue
        consecutive = 0
        rows = payload.get("artists") if isinstance(payload, dict) else None
        count = 0
        for position, raw in enumerate(rows or ()):
            before = len(merged)
            _merge_yt(merged, raw, cc, position)
            if len(merged) > before:
                count += 1
        sources_seen[f"chart:{cc}"] = {"ok": bool(rows), "n": len(rows or [])}
        if not rows:
            sources_seen[f"chart:{cc}"]["error"] = "empty"
        if progress is not None:
            progress(index + 1, total)
    if not merged and error is None:
        error = "empty"
    return _yt_snapshot(merged, sources_seen, error, started)


def _sources_ok(snapshot) -> int:
    return sum(1 for value in (snapshot.get("sources") or {}).values() if value.get("ok"))


def _acceptable(snapshot, old) -> bool:
    cards = len(snapshot.get("cards") or ())
    if cards >= MIN_POOL and _sources_ok(snapshot) >= MIN_SOURCES:
        return True
    return old is None and cards >= MIN_POOL


def _save(app, kind, snapshot) -> bool:
    old = get_pool(app, kind)
    accepted = _acceptable(snapshot, old)
    with _lock:
        _errors[kind] = snapshot.get("error")
        _progress[kind] = (0, 0)
        _building[kind] = False
        _attempted[kind] = True
    if not accepted:
        logger.info(
            "onboarding: kept the previous %s pool (%d cards); new build: %s",
            kind,
            len(_snapshot_cards(old)),
            snapshot.get("error") or "too thin",
        )
        return False
    stored = {
        key: value
        for key, value in snapshot.items()
        if key not in ("cards",)
    }
    _write_json(_pool_path(app, kind), stored)
    get_pool(app, kind)
    return True


def _worker(app, kind) -> None:
    try:
        workload.renice()
        with app.app_context():
            if kind == "deezer":
                snapshot = build_deezer(
                    public_client(),
                    progress=lambda done, total: _set_progress(kind, done, total),
                )
            else:
                snapshot = build_youtube(
                    progress=lambda done, total: _set_progress(kind, done, total),
                )
            _save(app, kind, snapshot)
    except Exception:
        logger.exception("onboarding: %s pool worker failed", kind)
        with _lock:
            _building[kind] = False
            _progress[kind] = (0, 0)
            _errors[kind] = _errors[kind] or "error"
            _attempted[kind] = True


def _set_progress(kind, done, total):
    with _lock:
        _progress[kind] = (int(done), int(total))


def _inline(app) -> bool:
    return bool((app.config.get("WEBAPP") or {}).get("onboarding_inline"))


def ensure_build(app, kind="deezer", force=False) -> bool:
    snapshot = get_pool(app, kind)
    if not force and _snapshot_cards(snapshot) and _fresh(snapshot):
        return False
    with _lock:
        if _building[kind] and not force:
            return False
        if not force and _attempted[kind] and snapshot is not None:
            return False
        _building[kind] = True
        _progress[kind] = (0, 0)
    thread = threading.Thread(
        target=_worker, args=(app, kind), name=f"onboarding-{kind}", daemon=True
    )
    thread.start()
    return True


def _library_count(user) -> int:
    try:
        return len(library_artists(user, limit=24))
    except Exception:
        return 0


def ensure_feed(app, user=None, library_count=None, force=False) -> bool:
    if app.testing and not _inline(app):
        return False
    if library_count is None:
        library_count = _library_count(user) if user is not None else 0
    deezer = get_pool(app, "deezer")
    if _snapshot_cards(deezer):
        if not _fresh(deezer) or force:
            return ensure_build(app, "deezer", force=force)
        return False
    if not deezer_enabled(app):
        with _lock:
            _errors["deezer"] = _errors["deezer"] or "disabled"
            _attempted["deezer"] = True
    elif force or not _attempted["deezer"]:
        return ensure_build(app, "deezer", force=force)
    if library_count >= 24:
        return False
    if not _youtube_usable(user):
        return False
    youtube = get_pool(app, "youtube")
    if _snapshot_cards(youtube):
        if _fresh(youtube):
            return False
        return ensure_build(app, "youtube")
    if not force and not _attempted["youtube"]:
        return ensure_build(app, "youtube")
    if force:
        return ensure_build(app, "youtube", force=True)
    return False


def warm_feed(app, user=None) -> None:
    try:
        with app.app_context():
            ensure_feed(app, user)
    except Exception:
        logger.debug("onboarding: warm feed skipped", exc_info=True)


def start(app) -> None:
    global _boot_started
    if app.testing or _boot_started:
        return
    _boot_started = True

    def boot():
        time.sleep(BOOT_DELAY)
        try:
            with app.app_context():
                snapshot = get_pool(app, "deezer")
                if deezer_enabled(app) and (
                    snapshot is None or not _fresh(snapshot)
                ):
                    ensure_build(app, "deezer")
        except Exception:
            logger.debug("onboarding: boot build skipped", exc_info=True)

    threading.Thread(target=boot, name="onboarding-boot", daemon=True).start()


def library_artists(user, limit=200) -> list[Card]:
    query = (
        Artist.select(
            Artist.id,
            Artist.name,
            Artist.deezer_id,
            fn.COUNT(Track.id).alias("tracks"),
            fn.SUM(Track.play_count).alias("plays"),
        )
        .join(Track, on=(Track.artist == Artist.id))
        .group_by(Artist.id, Artist.name, Artist.deezer_id)
        .order_by(fn.SUM(Track.play_count).desc(), fn.COUNT(Track.id).desc(), Artist.name)
        .limit(max(1, int(limit)))
    )
    if not getattr(user, "admin", False):
        query = query.where(Track.owner.is_null(True) | (Track.owner == user.id))
    cards = []
    seen = set()
    for index, row in enumerate(query.tuples()):
        artist_id, name, deezer_id, _tracks, _plays = row
        name = clean_name(name)
        if not name:
            continue
        if deezer_id:
            provider, ref = "deezer", str(deezer_id)
        else:
            provider, ref = "local", str(artist_id)
        card = Card(
            provider=provider,
            ref=ref,
            name=name,
            rank=index + 1,
            source="library",
            sources=("library",),
        )
        if card.key in seen:
            continue
        seen.add(card.key)
        cards.append(card)
    return cards


def feed_cards(app, user):
    deezer = get_pool(app, "deezer")
    cards = _snapshot_cards(deezer)
    if cards:
        return cards, "deezer", 0
    local = library_artists(user, limit=200)
    if len(local) >= 24:
        return local, "library", len(local)
    youtube = _snapshot_cards(get_pool(app, "youtube")) if _youtube_usable(user) else []
    if youtube:
        return local + youtube, "youtube", len(local)
    return local, "library", len(local)


def feed_state(app, user) -> dict:
    deezer = get_pool(app, "deezer")
    cards = _snapshot_cards(deezer)
    if cards:
        source, ready, library_count = "deezer", True, 0
    else:
        local = library_artists(user, limit=24)
        library_count = len(local)
        if library_count >= 24:
            source, ready = "library", True
        else:
            youtube = (
                _snapshot_cards(get_pool(app, "youtube"))
                if _youtube_usable(user)
                else []
            )
            if youtube:
                source, ready = "youtube", True
            else:
                source, ready = "library", bool(local)
    with _lock:
        building = _building["deezer"] or _building["youtube"]
        error = _errors["deezer"] or _errors["youtube"]
    state = state_of(app, "deezer") if source == "deezer" else (
        state_of(app, "youtube") if source == "youtube" else state_of(app, "deezer")
    )
    if state.get("error"):
        error = state["error"]
    if source == "youtube":
        kept = library_count + len(_snapshot_cards(get_pool(app, "youtube")))
    elif source == "deezer":
        kept = len(cards)
    else:
        kept = library_count
    return {
        "ready": ready,
        "building": building,
        "source": source,
        "error": error,
        "state": "ready" if ready else ("building" if building else "unavailable"),
        "build": state.get("build"),
        "stale": state.get("stale"),
        "kept": kept,
        "progress": state.get("progress"),
        "degraded": source != "deezer",
        "library_count": library_count,
    }


def account_state(app, user) -> dict:
    providers = set()
    try:
        providers = {
            provider
            for (provider,) in ProviderAccount.select(ProviderAccount.provider)
            .where(ProviderAccount.user == user)
            .tuples()
        }
    except Exception:
        logger.debug("Could not read the account's providers", exc_info=True)
    youtube = youtube_status(user)
    deezer_available = deezer_enabled(app)
    return {
        "deezer": {
            "available": deezer_available,
            "linked": "deezer" in providers,
            "auth_modes": ["arl", "native"] if deezer_available else [],
        },
        "youtube": {
            "available": bool(youtube["enabled"]),
            "linked": "youtube" in providers,
            "auth_modes": ["headers"] if youtube["enabled"] else [],
        },
        "imports": [
            {"source": "deezer", "label": "Deezer", "mode": "link"},
            {"source": "youtube", "label": "YouTube Music", "mode": "link"},
            {"source": "spotify", "label": "Spotify", "mode": "public_link"},
            {"source": "apple", "label": "Apple Music", "mode": "public_link"},
        ],
        "can_save_imports": True,
    }


def _cached_card(app, provider, ref):
    card = artist_cache.get(provider, ref)
    if card is not None:
        return card
    snapshot = get_pool(app, "deezer" if provider == "deezer" else "youtube")
    for candidate in _snapshot_cards(snapshot):
        if candidate.provider == provider and candidate.ref == str(ref):
            artist_cache.remember([candidate])
            return candidate
    return None


def remember_cards(cards) -> None:
    artist_cache.remember(cards)


def card_by_entity(app, entity_id):
    card = artist_cache.by_entity(entity_id)
    if card is not None:
        return card
    for candidate in _snapshot_cards(get_pool(app, "youtube")):
        if candidate.entity_id == str(entity_id):
            artist_cache.remember([candidate])
            return candidate
    return None


def card_by_ref(app, provider, ref):
    card = artist_cache.get(provider, ref)
    if card is not None:
        return card
    kind = "deezer" if provider == "deezer" else "youtube"
    for candidate in _snapshot_cards(get_pool(app, kind)):
        if candidate.provider == provider and candidate.ref == str(ref):
            artist_cache.remember([candidate])
            return candidate
    return None


def related_links(app, ref):
    rows = list(
        ArtistLink.select()
        .where((ArtistLink.provider == "deezer") & (ArtistLink.artist_ref == str(ref)))
        .order_by(ArtistLink.pos)
        .limit(RELATED_MAX)
    )
    if not rows:
        return [], False, False
    fresh = all(row.fetched and now() - row.fetched <= timedelta(seconds=RELATED_TTL) for row in rows)
    cards = []
    for row in rows:
        cached = _cached_card(app, "deezer", row.other_ref)
        cards.append(
            Card(
                provider="deezer",
                ref=str(row.other_ref),
                name=clean_name(row.other_name) or row.other_name,
                picture=row.other_picture,
                picture_small=row.other_picture,
                genres=cached.genres if cached else (),
                countries=cached.countries if cached else (),
                rank=row.pos,
                source="related",
                sources=("related",),
            )
        )
    return cards, fresh, not fresh


def _related_worker(app) -> None:
    global _related_running
    workload.renice()
    with app.app_context():
        while True:
            with _related_lock:
                if not _related_queue:
                    _related_running = False
                    return
                ref = _related_queue.popleft()
                _related_queued.discard(ref)
            try:
                _run_related(app, ref)
            except Exception:
                logger.warning("onboarding: related worker failed for %s", ref, exc_info=True)
                with _related_lock:
                    _related_status[ref] = {"state": "error", "error": "error"}


def _run_related(app, ref) -> None:
    client = public_client()
    if client is None:
        with _related_lock:
            _related_status[ref] = {"state": "error", "error": "deezer-unreachable"}
        return
    try:
        payload = client.get_artist_related(str(ref), 0, RELATED_MAX)
    except Exception as exc:
        from deezerpy.errors import is_transport_failure

        code = "deezer-unreachable" if is_transport_failure(exc) else "error"
        with _related_lock:
            _related_status[ref] = {"state": "error", "error": code}
        return
    rows = payload.get("data") if isinstance(payload, dict) else payload
    rows = [row for row in (rows or ()) if isinstance(row, dict) and valid_deezer_ref(row.get("id"))]
    if not rows:
        with _related_lock:
            _related_status[ref] = {"state": "error", "error": "empty"}
        return
    cards = []
    link_rows = []
    for position, row in enumerate(rows[:RELATED_MAX], start=1):
        other = str(row["id"])
        name = clean_name(row.get("name"))
        if not name:
            continue
        picture = row.get("picture_medium") or row.get("picture_big") or row.get("picture")
        link_rows.append(
            {
                "provider": "deezer",
                "artist_ref": str(ref),
                "other_ref": other,
                "other_name": name[:255],
                "other_picture": (str(picture)[:512] if picture else None),
                "score": 1.0 - (position - 1) / float(RELATED_MAX),
                "pos": position,
                "fetched": now(),
            }
        )
        cards.append(
            Card(
                provider="deezer",
                ref=other,
                name=name,
                picture=(str(picture) if picture else None),
                picture_small=(str(picture) if picture else None),
                rank=position,
                source="related",
                sources=("related",),
            )
        )
    if not link_rows:
        with _related_lock:
            _related_status[ref] = {"state": "error", "error": "empty"}
        return
    from ..db import db

    with db.atomic():
        ArtistLink.delete().where(
            (ArtistLink.provider == "deezer") & (ArtistLink.artist_ref == str(ref))
        ).execute()
        ArtistLink.insert_many(link_rows).execute()
    artist_cache.remember(cards)
    with _related_lock:
        _related_status.pop(ref, None)


def request_related(app, ref) -> dict:
    ref = str(ref)
    with _related_lock:
        if ref in _related_queued:
            return {"state": "pending", "retry_after": 1}
        if len(_related_queue) >= RELATED_QUEUE:
            _related_status[ref] = {"state": "error", "error": "busy"}
            return {"state": "error", "error": "busy", "retry_after": 30}
        _related_queue.append(ref)
        _related_queued.add(ref)
        global _related_running
        start_worker = not _related_running
        if start_worker:
            _related_running = True
    if app.testing and not _inline(app):
        return {"state": "pending", "retry_after": 1}
    if app.testing and _inline(app):
        _run_related(app, ref)
        return related_state(app, ref)
    if start_worker:
        threading.Thread(
            target=_related_worker, args=(app,), name="onboarding-related", daemon=True
        ).start()
    return {"state": "pending", "retry_after": 1}


def run_related_now(app, ref) -> None:
    with _related_lock:
        _related_status.pop(str(ref), None)
    with app.app_context():
        _run_related(app, str(ref))


def related_state(app, ref) -> dict:
    with _related_lock:
        status = _related_status.get(str(ref))
    if status is not None:
        return dict(status)
    return {"state": "pending", "retry_after": 1}


def similar_payload(app, user, card, count=4, seen=()) -> dict:
    if card.provider != "deezer":
        return {
            "state": "none",
            "items": [],
            "complete": False,
            "error": None,
            "retry_after": None,
        }
    links, fresh, stale = related_links(app, card.ref)
    if links:
        excluded = {card.api_id, card.ref, *[str(value) for value in seen]}
        picked = pick_similar(links, exclude_refs=excluded, count=count)
        items = [
            item.as_item(via="related", parent=card.ref) for item in picked
        ]
        hints = page_hints(picked)
        for item, picked_card in zip(items, picked):
            item["hint"] = hints.get(picked_card.key)
        return {
            "state": "ready",
            "items": items,
            "complete": len(items) >= 3,
            "error": None,
            "retry_after": None,
            "cached": bool(fresh),
            "stale": bool(stale),
        }
    status = related_state(app, card.ref)
    if status.get("state") == "error":
        return {
            "state": "error",
            "items": [],
            "complete": False,
            "error": status.get("error"),
            "retry_after": 30 if status.get("error") == "busy" else 15,
        }
    request_related(app, card.ref)
    return {
        "state": "pending",
        "items": [],
        "complete": False,
        "error": None,
        "retry_after": 1,
    }


def deezer_search(query, limit=25):
    client = public_client()
    if client is None:
        raise PoolError("absent")
    try:
        payload = client.search_artist(query, 0, limit)
    except Exception as exc:
        from deezerpy.errors import is_transport_failure

        raise PoolError("deezer-unreachable" if is_transport_failure(exc) else "error")
    rows = payload.get("data") if isinstance(payload, dict) else payload
    cards = []
    for row in rows or ():
        if not isinstance(row, dict) or not valid_deezer_ref(row.get("id")):
            continue
        name = clean_name(row.get("name"))
        if not name:
            continue
        cards.append(
            Card(
                provider="deezer",
                ref=str(row["id"]),
                name=name,
                picture=row.get("picture_medium") or row.get("picture_big") or row.get("picture"),
                picture_small=row.get("picture_small") or row.get("picture_medium"),
                source="search",
                sources=("search",),
                fans=max(0, int(row.get("nb_fan") or 0)),
            )
        )
    return cards


def youtube_search(query, limit=20):
    if not _youtube_usable():
        raise PoolError("youtube-absent")
    client = None
    try:
        client = yt._client()
        yt._check_held()
        rows = client.search(query, filter="artists", limit=20)
    except yt.YoutubeHeld:
        raise PoolError("youtube-held")
    except Exception as exc:
        yt._hold_for(exc)
        raise PoolError("youtube-error")
    cards = []
    for index, row in enumerate(rows or ()):
        if not isinstance(row, dict):
            continue
        card = _yt_card(row, None, index)
        if card is None:
            continue
        cards.append(dataclasses_replace(card))
        if len(cards) >= limit:
            break
    return cards


def dataclasses_replace(card):
    return replace(
        card,
        source="youtube-search",
        sources=("youtube-search",),
        countries=(),
    )


def local_search(user, query, limit=20) -> list[Card]:
    folded = fold_name(query)
    if not folded:
        return []
    query_obj = (
        Artist.select(
            Artist.id,
            Artist.name,
            Artist.deezer_id,
            fn.MAX(Track.play_count).alias("plays"),
        )
        .join(Track, on=(Track.artist == Artist.id))
        .group_by(Artist.id, Artist.name, Artist.deezer_id)
        .order_by(fn.MAX(Track.play_count).desc(), Artist.name)
        .limit(500)
    )
    if not getattr(user, "admin", False):
        query_obj = query_obj.where(Track.owner.is_null(True) | (Track.owner == user.id))
    cards = []
    for row in query_obj.tuples():
        artist_id, name, deezer_id, _plays = row
        name = clean_name(name)
        if not name or folded not in fold_name(name):
            continue
        if deezer_id:
            provider, ref = "deezer", str(deezer_id)
        else:
            provider, ref = "local", str(artist_id)
        cards.append(
            Card(
                provider=provider,
                ref=ref,
                name=name,
                source="library",
                sources=("library",),
            )
        )
        if len(cards) >= limit:
            break
    return cards


def pool_search(app, query) -> list[Card]:
    folded = fold_name(query)
    if not folded:
        return []
    out = []
    for kind in ("deezer", "youtube"):
        for card in _snapshot_cards(get_pool(app, kind)):
            if folded in fold_name(card.name):
                out.append(card)
    return out


def _picture_card(app, entity_id):
    if not valid_uuid(entity_id):
        return None
    card = artist_cache.by_entity(entity_id)
    if card is None:
        for candidate in _snapshot_cards(get_pool(app, "youtube")):
            if candidate.entity_id == str(entity_id):
                card = candidate
                artist_cache.remember([card])
                break
    if card is None:
        try:
            import uuid as _uuid

            from ..db import EntityRef

            row = (
                EntityRef.select()
                .where(
                    (EntityRef.kind == "artist")
                    & (EntityRef.provider == "youtube")
                    & (EntityRef.entity == _uuid.UUID(str(entity_id)))
                )
                .first()
            )
        except Exception:
            row = None
        if row is not None:
            for candidate in _snapshot_cards(get_pool(app, "youtube")):
                if candidate.ref == row.ref:
                    card = candidate
                    artist_cache.remember([card])
                    break
    return card if card is not None and card.picture else None


def picture(app, entity_id, size=250, fetch=True):
    size = 60 if int(size or 250) == 60 else 250
    card = _picture_card(app, entity_id)
    if card is None:
        return None
    url = _yt_picture(card.picture, size)
    if not url:
        return None
    directory = os.path.join(_root(app), "pictures")
    os.makedirs(directory, exist_ok=True)
    digest = hashlib.sha256(
        f"{card.ref}:{size}".encode("utf-8")
    ).hexdigest()[:40]
    for extension, mimetype in ((".jpg", "image/jpeg"), (".png", "image/png")):
        path = os.path.join(directory, digest + extension)
        if os.path.isfile(path):
            return path, mimetype
    if not fetch:
        return None
    failure_key = f"{card.ref}:{size}"
    with _picture_lock:
        if _picture_failures.get(failure_key, 0.0) > time.monotonic():
            return None
    status = youtube_status()
    if status["error"] in ("absent", "disabled", "held", "denied"):
        return None
    try:
        with requests.get(
            url,
            timeout=(5, 20),
            allow_redirects=False,
            stream=True,
            headers={"User-Agent": "Neezer/1 onboarding"},
        ) as response:
            if response.status_code != 200:
                return None
            content_type = (response.headers.get("Content-Type") or "").split(";")[0].strip()
            if content_type not in ("image/jpeg", "image/png"):
                return None
            length = int(response.headers.get("Content-Length") or 0)
            if length > PICTURE_MAX:
                return None
            data = bytearray()
            for chunk in response.iter_content(64 * 1024):
                data.extend(chunk)
                if len(data) > PICTURE_MAX:
                    return None
            if content_type == "image/jpeg" and bytes(data[:2]) != b"\xff\xd8":
                return None
            if content_type == "image/png" and bytes(data[:4]) != b"\x89PNG":
                return None
            extension = ".jpg" if content_type == "image/jpeg" else ".png"
            path = os.path.join(directory, digest + extension)
            temporary = path + ".tmp"
            with open(temporary, "wb") as handle:
                handle.write(bytes(data))
                handle.flush()
                os.fsync(handle.fileno())
            os.replace(temporary, path)
            return path, content_type
    except Exception:
        logger.debug("onboarding: picture fetch failed", exc_info=True)
        with _picture_lock:
            _picture_failures[failure_key] = time.monotonic() + 60.0
        return None


_picture_fetching = set()


def request_picture(app, entity_id, size=250):
    """Start one background download of a portrait, never in the request."""
    if _picture_card(app, entity_id) is None:
        return False
    key = (str(entity_id), int(size))
    with _picture_lock:
        if key in _picture_fetching:
            return False
        _picture_fetching.add(key)

    def work():
        try:
            picture(app, entity_id, size)
        finally:
            with _picture_lock:
                _picture_fetching.discard(key)

    threading.Thread(target=work, name="onboarding-picture", daemon=True).start()
    return True


def install_snapshot(app, kind, cards, built_at=None, sources_map=None) -> None:
    moment = built_at or now()
    snapshot = {
        "v": POOL_VERSION,
        "build": _build_id(moment),
        "built_at": moment.isoformat(),
        "sources": sources_map
        or {
            "genres": {"ok": True, "n": 1},
            "chart": {"ok": True, "n": len(cards)},
            "country:FR": {"ok": True, "n": len(cards)},
        },
        "artists": [card.as_dict() for card in cards],
        "partial": False,
        "error": None,
    }
    _write_json(_pool_path(app, kind), snapshot)
    with _lock:
        _loaded.pop(kind, None)
        _errors[kind] = None
        _attempted[kind] = True


def build_now(app, kind="deezer", client=None) -> dict:
    with app.app_context():
        workload.renice()
        if kind == "deezer":
            snapshot = build_deezer(client if client is not None else public_client())
        else:
            snapshot = build_youtube(client)
        _save(app, kind, snapshot)
        return snapshot
