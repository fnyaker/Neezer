# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""The onboarding artist routes: feed, search, like/unlike, similar, steps.

Every route is ``@login_required`` and same-origin guarded. A like is a local
taste write (``Artist`` + ``StarredArtist``, one mute removed); it never
downloads, archives, prefetches or writes to a provider. A cold similar cache
answers ``pending`` and starts one worker; no route waits on a provider.
"""

from __future__ import annotations

import logging
import operator
import uuid
from functools import reduce

from flask import Blueprint, jsonify, request, send_file
from peewee import IntegrityError

from .. import discover, sources
from ..db import Artist, ArtistMute, EntityRef, StarredArtist, now
from ..deezer import ids, library
from ..deezer import onboarding_pool as pool
from ..onboarding import (
    DIVERSITY,
    MAX_FEED,
    PAGE,
    PAGE_MAX,
    Card,
    clean_name,
    diversify,
    fold_name,
    next_after,
    page_hints,
    rank_search,
    read_after,
    search_cache,
    seed_for,
    valid_deezer_ref,
    valid_uuid,
)
from ..ratelimit import WindowLimiter
from . import login_required, same_origin_guard
from .remote import NEVER, POLICY

logger = logging.getLogger(__name__)

MAX_BODY = 8 * 1024
SEARCH_DEFAULT = 25
SEARCH_MAX = 50
SEARCH_MIN_CHARS = 2
SEEN_MAX = 200
SIMILAR_DEFAULT = 4

search_limiter = WindowLimiter(40, 60)
retry_limiter = WindowLimiter(1, 300)
like_limiter = WindowLimiter(60, 3600)

ONBOARDING_POLICY = {
    "onboarding_get": NEVER,
    "onboarding_artists": NEVER,
    "onboarding_search": NEVER,
    "onboarding_like": NEVER,
    "onboarding_unlike": NEVER,
    "onboarding_related": NEVER,
    "onboarding_artists_done": NEVER,
    "onboarding_account_done": NEVER,
    "onboarding_feed_retry": NEVER,
    "onboarding_picture": NEVER,
}


class _OnboardingBlueprint(Blueprint):
    def register(self, app, options):
        super().register(app, options)
        POLICY.update(ONBOARDING_POLICY)


onboarding_api = _OnboardingBlueprint("onboarding_api", __name__)


@onboarding_api.before_request
def reject_cross_site():
    request.max_content_length = MAX_BODY
    return same_origin_guard()


@onboarding_api.errorhandler(Exception)
def _json_error(exc):
    from werkzeug.exceptions import HTTPException

    if isinstance(exc, HTTPException):
        return jsonify({"error": exc.name}), exc.code
    logger.error("Unhandled error on %s %s", request.method, request.path, exc_info=True)
    return jsonify({"error": "server error"}), 500


class _Refused(Exception):
    def __init__(self, status, code, message, retry_after=None):
        super().__init__(message)
        self.status = status
        self.code = code
        self.message = message
        self.retry_after = retry_after

    def response(self):
        body = {"error": self.message, "code": self.code}
        response = jsonify(body)
        response.status_code = self.status
        if self.retry_after:
            response.headers["Retry-After"] = str(int(self.retry_after))
        if self.status == 503 and self.code == "deezer-unreachable":
            response.headers["X-NS-Error"] = "deezer-unreachable"
        return response


@onboarding_api.errorhandler(_Refused)
def _refused_response(exc):
    return exc.response()


def _bad(message="Paramètre invalide.", code="invalid_request"):
    return _Refused(400, code, message)


def _not_found(message="Artiste introuvable.", code="not_found"):
    return _Refused(404, code, message)


def _unavailable(code, message):
    return _Refused(503, code, message, retry_after=30)


def _body():
    if request.content_length and request.content_length > MAX_BODY:
        return None, _Refused(413, "too_large", "Requête trop volumineuse.")
    data = request.get_json(silent=True)
    if data is None:
        return {}, None
    if not isinstance(data, dict):
        return None, _bad()
    return data, None


def _step(user) -> str:
    step = getattr(user, "onboarding_step", None) or "done"
    return step if step in ("artists", "account", "done") else "done"


def _done_guard(user):
    if _step(user) == "done":
        return _Refused(
            409, "onboarding_done", "L'onboarding est terminé."
        )
    return None


def _set_step(user, step):
    from ..db import User

    User.update(onboarding_step=step).where(User.id == user.id).execute()
    user.onboarding_step = step


def _starred_uuid(raw):
    if valid_deezer_ref(raw):
        return ids.artist_uuid(raw)
    if valid_uuid(raw):
        return uuid.UUID(str(raw))
    return None


def _starred_ids(user, cards):
    uuids = []
    for card in cards:
        try:
            uuids.append(uuid.UUID(card.entity_id))
        except ValueError:
            continue
    if not uuids:
        return set()
    rows = (
        StarredArtist.select(StarredArtist.starred)
        .where(StarredArtist.user == user, StarredArtist.starred.in_(uuids))
        .tuples()
    )
    return {str(value) for (value,) in rows}


def _mute_keys(user):
    rows = (
        ArtistMute.select(ArtistMute.provider, ArtistMute.ref)
        .where(ArtistMute.user == user)
        .tuples()
    )
    return {(str(provider), str(ref)) for provider, ref in rows}


def _muted(card, mutes):
    keys = {(card.provider, card.ref), ("local", card.entity_id)}
    if card.provider == "deezer":
        keys.add(("deezer", card.ref))
    return bool(keys & mutes)


def _build_flags(app):
    deezer = pool.state_of(app, "deezer")
    youtube = pool.state_of(app, "youtube")
    return (
        bool(deezer.get("building") or youtube.get("building")),
        deezer.get("error") or youtube.get("error"),
        deezer if deezer.get("ready") else youtube,
    )


def _card_for_uuid(app, raw):
    card = pool.artist_cache.by_entity(raw)
    if card is not None:
        return card
    card = pool.card_by_entity(app, raw)
    if card is not None:
        return card
    try:
        row = Artist.get_or_none(Artist.id == uuid.UUID(str(raw)))
    except ValueError:
        row = None
    if row is None:
        raise _not_found()
    if row.deezer_id:
        return Card(
            provider="deezer",
            ref=str(row.deezer_id),
            name=row.name,
            source="library",
            sources=("library",),
        )
    return Card(
        provider="local",
        ref=str(row.id),
        name=row.name,
        source="library",
        sources=("library",),
    )


def _card_for_deezer(app, raw):
    card = pool.card_by_ref(app, "deezer", raw)
    if card is not None:
        return card
    row = (
        Artist.select()
        .where((Artist.deezer_id == str(raw)) | (Artist.id == ids.artist_uuid(raw)))
        .first()
    )
    if row is not None:
        card = Card(
            provider="deezer",
            ref=str(raw),
            name=row.name,
            source="library",
            sources=("library",),
        )
        pool.remember_cards([card])
        return card
    client = pool.public_client()
    if client is None:
        raise _unavailable("deezer-unreachable", "Deezer est injoignable.")
    try:
        payload = client.get_artist(str(raw))
    except Exception as exc:
        from deezerpy.errors import is_transport_failure

        if is_transport_failure(exc):
            raise _unavailable("deezer-unreachable", "Deezer est injoignable.")
        raise _not_found()
    name = clean_name((payload or {}).get("name"))
    if not name:
        raise _not_found()
    card = Card(
        provider="deezer",
        ref=str(raw),
        name=name,
        picture=(payload or {}).get("picture_medium") or (payload or {}).get("picture_big"),
        picture_small=(payload or {}).get("picture_small"),
        source="search",
        sources=("search",),
    )
    pool.remember_cards([card])
    return card


def _cached_card(app, raw):
    if valid_deezer_ref(raw):
        card = pool.card_by_ref(app, "deezer", raw)
        if card is not None:
            return card
        row = (
            Artist.select()
            .where(
                (Artist.deezer_id == str(raw))
                | (Artist.id == ids.artist_uuid(raw))
            )
            .first()
        )
        if row is None:
            raise _not_found()
        card = Card(
            provider="deezer",
            ref=str(raw),
            name=row.name,
            source="library",
            sources=("library",),
        )
        pool.remember_cards([card])
        return card
    if valid_uuid(raw):
        return _card_for_uuid(app, raw)
    raise _bad("Identifiant d'artiste invalide.", "invalid_artist")


def _resolve_card(app, raw):
    if valid_deezer_ref(raw):
        return _card_for_deezer(app, raw)
    if valid_uuid(raw):
        return _card_for_uuid(app, raw)
    raise _bad("Identifiant d'artiste invalide.", "invalid_artist")


def _row_for_card(card):
    if card.provider == "deezer":
        uid = ids.artist_uuid(card.ref)
        existing = Artist.get_or_none(
            (Artist.deezer_id == card.ref) | (Artist.id == uid)
        )
        if existing is not None:
            if existing.name != card.name:
                card = Card(
                    provider=card.provider,
                    ref=card.ref,
                    name=existing.name,
                    picture=card.picture,
                    picture_small=card.picture_small,
                    genres=card.genres,
                    countries=card.countries,
                    rank=card.rank,
                    source=card.source,
                    sources=card.sources,
                    fans=card.fans,
                )
            return existing, card
        row = library.create_or_get(
            lambda: Artist.create(id=uid, name=card.name, deezer_id=card.ref),
            lambda: Artist.get_or_none(Artist.id == uid),
        )
        if row is None:
            raise _unavailable("deezer-unreachable", "Artiste indisponible.")
        sources.attach_entity("artist", "deezer", card.ref, row.id)
        return row, card
    if card.provider == "youtube":
        mapping = EntityRef.get_or_none(
            (EntityRef.kind == "artist")
            & (EntityRef.provider == "youtube")
            & (EntityRef.ref == card.ref)
        )
        if mapping is not None:
            row = Artist.get_or_none(Artist.id == mapping.entity)
            if row is not None:
                return row, card
        uid = ids.youtube_artist_uuid(card.ref)
        row = Artist.get_or_none(Artist.id == uid)
        if row is None:
            row = library.create_or_get(
                lambda: Artist.create(id=uid, name=card.name, deezer_id=None),
                lambda: Artist.get_or_none(Artist.id == uid),
            )
            if row is not None:
                sources.attach_entity("artist", "youtube", card.ref, row.id)
        if row is None:
            raise _unavailable("deezer-unreachable", "Artiste indisponible.")
        return row, card
    row = Artist.get_or_none(Artist.id == uuid.UUID(str(card.ref)))
    if row is None:
        raise _not_found()
    return row, card


def _unmute(user, card, row):
    conditions = []
    if card.provider == "deezer":
        conditions.append((ArtistMute.provider == "deezer") & (ArtistMute.ref == card.ref))
        conditions.append((ArtistMute.provider == "local") & (ArtistMute.ref == str(row.id)))
    elif card.provider == "youtube":
        conditions.append((ArtistMute.provider == "youtube") & (ArtistMute.ref == card.ref))
        conditions.append((ArtistMute.provider == "local") & (ArtistMute.ref == str(row.id)))
    else:
        conditions.append((ArtistMute.provider == "local") & (ArtistMute.ref == str(row.id)))
    ArtistMute.delete().where(
        (ArtistMute.user == user) & reduce(operator.or_, conditions)
    ).execute()


def _artist_payload(card, row):
    return {
        "deezer_id": card.api_id,
        "id": card.api_id,
        "name": row.name,
        "picture": card.picture_url(),
        "picture_small": card.picture_url(small=True),
        "provider": card.provider,
        "openable": card.openable,
    }


def _search_items(cards):
    items = []
    for card in cards:
        item = card.as_item()
        item["fans"] = card.fans or None
        items.append(item)
    return items


def _search_payload(app, user, raw_query, limit):
    raw_query = " ".join(str(raw_query or "").split())[:80]
    folded = fold_name(raw_query)
    if not folded:
        return {
            "query": "",
            "items": [],
            "local": 0,
            "partial": False,
            "error": None,
        }
    allowed, retry_after = search_limiter.allow(f"u:{user.id}")
    if not allowed:
        raise _Refused(
            429, "rate_limited", "Trop de recherches. Réessaie dans un moment.", retry_after
        )
    cache_key = (user.id, folded, int(limit))
    cached = search_cache.get(cache_key)
    if cached is not None:
        return dict(cached)
    local_cards = pool.pool_search(app, raw_query)
    seen = {card.key for card in local_cards}
    for card in pool.local_search(user, raw_query, limit=20):
        if card.key not in seen:
            seen.add(card.key)
            local_cards.append(card)
    local_ranked = rank_search(local_cards, [], raw_query)
    pool.remember_cards(local_ranked)
    local_count = len(local_ranked)
    if len(folded) < SEARCH_MIN_CHARS:
        payload = {
            "query": raw_query,
            "items": _search_items(local_ranked),
            "local": local_count,
            "partial": False,
            "error": None,
            "local_only": True,
        }
        search_cache.put(cache_key, payload, 60)
        return dict(payload)
    remote = []
    error = None
    try:
        remote = pool.deezer_search(raw_query, limit)
    except pool.PoolError as exc:
        error = exc.code
        if len(local_ranked) < 3 and pool._youtube_usable(user):
            try:
                remote = pool.youtube_search(raw_query, min(limit, 20))
                if remote:
                    error = None
            except pool.PoolError as youtube_exc:
                error = youtube_exc.code
    pool.remember_cards(remote)
    ranked = rank_search(local_ranked, remote, raw_query)[:limit]
    payload = {
        "query": raw_query,
        "items": _search_items(ranked),
        "local": local_count,
        "partial": error is not None,
        "error": error,
    }
    ttl = 300 if not ranked else (60 if error is None else 60)
    search_cache.put(cache_key, payload, ttl)
    return dict(payload)


@onboarding_api.route("/onboarding")
@login_required
def onboarding_get():
    user = request.webuser
    liked = StarredArtist.select().where(StarredArtist.user == user).count()
    feed = pool.feed_state(request_app(), user)
    return jsonify(
        {
            "step": _step(user),
            "liked": liked,
            "min_artists": 1,
            "diversity": DIVERSITY,
            "feed": {
                "ready": feed["ready"],
                "building": feed["building"],
                "source": feed["source"],
                "error": feed["error"],
                "state": feed["state"],
                "build": feed["build"],
                "stale": feed["stale"],
                "kept": feed["kept"],
                "progress": feed["progress"],
            },
            "account": pool.account_state(request_app(), user),
            "fallback": None if feed["source"] == "deezer" else feed["source"],
            "youtube": pool.youtube_status(user),
        }
    )


def request_app():
    from flask import current_app

    return current_app._get_current_object()


@onboarding_api.route("/onboarding/artists")
@login_required
def onboarding_artists():
    user = request.webuser
    after = read_after(request.args.get("after", request.args.get("cursor")))
    if after is None:
        raise _bad("Curseur invalide.", "invalid_cursor")
    try:
        limit = int(request.args.get("limit") or PAGE)
    except (TypeError, ValueError):
        raise _bad("Limite invalide.", "invalid_limit")
    if not 1 <= limit <= PAGE_MAX:
        raise _bad("Limite invalide.", "invalid_limit")
    query = request.args.get("q")
    if query:
        payload = _search_payload(request_app(), user, query, SEARCH_MAX)
        payload["next"] = None
        payload["source"] = "search"
        payload["building"] = False
        return jsonify(payload)
    app = request_app()
    cards, source, library_count = pool.feed_cards(app, user)
    pool.ensure_feed(app, user, library_count=library_count)
    build = (
        pool.state_of(app, "deezer").get("build")
        if source == "deezer"
        else pool.state_of(app, "youtube").get("build")
    )
    seed = seed_for(user.id, build or "library")
    ordered = diversify(cards, seed, limit=MAX_FEED)
    page = ordered[after : after + limit]
    starred = _starred_ids(user, page)
    mutes = _mute_keys(user)
    visible = [
        card
        for card in page
        if card.entity_id not in starred and not _muted(card, mutes)
    ]
    pool.remember_cards(visible)
    hints = page_hints(visible)
    building, error, state = _build_flags(app)
    return jsonify(
        {
            "items": [
                card.as_item(
                    liked=False,
                    hint=hints.get(card.key),
                )
                for card in visible
            ],
            "next": next_after(after + limit, len(ordered)),
            "source": source,
            "degraded": source != "deezer",
            "building": building,
            "error": error,
            "state": "ready" if cards else ("building" if building else "unavailable"),
            "total": min(len(ordered), MAX_FEED),
            "build": build,
            "seed": seed,
            "partial": state.get("partial") if state else False,
        }
    )


@onboarding_api.route("/onboarding/search")
@login_required
def onboarding_search():
    try:
        limit = int(request.args.get("limit") or SEARCH_DEFAULT)
    except (TypeError, ValueError):
        raise _bad("Limite invalide.", "invalid_limit")
    if not 1 <= limit <= SEARCH_MAX:
        raise _bad("Limite invalide.", "invalid_limit")
    return jsonify(_search_payload(request_app(), request.webuser, request.args.get("q"), limit))


@onboarding_api.route("/onboarding/like", methods=["POST"])
@onboarding_api.route("/onboarding/artists/like", methods=["POST"])
@login_required
def onboarding_like():
    user = request.webuser
    instruction = _done_guard(user)
    if instruction is not None:
        raise instruction
    data, failure = _body()
    if failure is not None:
        raise failure
    allowed, retry_after = like_limiter.allow(f"u:{user.id}")
    if not allowed:
        raise _Refused(
            429, "rate_limited", "Trop d'artistes suivis d'un coup. Réessaie plus tard.", retry_after
        )
    raw = str(data.get("deezer_id") or data.get("id") or "")
    app = request_app()
    try:
        card = _resolve_card(app, raw)
        row, card = _row_for_card(card)
    except _Refused:
        raise
    except Exception:
        logger.warning("onboarding: like failed for %r", raw, exc_info=True)
        raise _not_found()
    try:
        StarredArtist.create(user=user, starred=row, date=now())
    except IntegrityError:
        pass
    _unmute(user, card, row)
    discover.invalidate(user)
    seen = data.get("seen") if isinstance(data.get("seen"), list) else ()
    seen = [str(value) for value in seen[:SEEN_MAX]]
    related = pool.similar_payload(app, user, card, seen=seen)
    return jsonify(
        {
            "liked": True,
            "artist": _artist_payload(card, row),
            "related": related,
        }
    )


@onboarding_api.route("/onboarding/unlike", methods=["POST"])
@onboarding_api.route("/onboarding/artists/unlike", methods=["POST"])
@onboarding_api.route("/onboarding/artists/<artist_id>/like", methods=["DELETE"])
@login_required
def onboarding_unlike(artist_id=None):
    user = request.webuser
    instruction = _done_guard(user)
    if instruction is not None:
        raise instruction
    raw = artist_id
    if raw is None:
        data, failure = _body()
        if failure is not None:
            raise failure
        raw = str(data.get("deezer_id") or data.get("id") or "")
    starred = _starred_uuid(raw)
    if starred is None:
        raise _bad("Identifiant d'artiste invalide.", "invalid_artist")
    StarredArtist.delete().where(
        (StarredArtist.user == user) & (StarredArtist.starred == starred)
    ).execute()
    discover.invalidate(user)
    return jsonify({"liked": False})


@onboarding_api.route("/onboarding/related/<artist_id>")
@onboarding_api.route("/onboarding/artists/<artist_id>/related")
@login_required
def onboarding_related(artist_id):
    user = request.webuser
    app = request_app()
    if valid_deezer_ref(artist_id):
        card = _cached_card(app, artist_id)
        starred = ids.artist_uuid(artist_id)
    elif valid_uuid(artist_id):
        card = _card_for_uuid(app, artist_id)
        starred = uuid.UUID(str(artist_id))
    else:
        raise _bad("Identifiant d'artiste invalide.", "invalid_artist")
    if StarredArtist.get_or_none(
        (StarredArtist.user == user) & (StarredArtist.starred == starred)
    ) is None:
        raise _not_found("Cet artiste n'est pas suivi.", "not_starred")
    seen = []
    for value in (request.args.get("seen") or "").split(","):
        value = value.strip()
        if value and len(seen) < SEEN_MAX:
            seen.append(value)
    payload = pool.similar_payload(app, user, card, seen=seen)
    payload.setdefault("cached", payload.get("state") == "ready")
    return jsonify(payload)


@onboarding_api.route("/onboarding/artists/done", methods=["POST"])
@login_required
def onboarding_artists_done():
    user = request.webuser
    step = _step(user)
    if step in ("account", "done"):
        return jsonify({"step": step})
    data, failure = _body()
    if failure is not None:
        raise failure
    count = StarredArtist.select().where(StarredArtist.user == user).count()
    if data.get("skip"):
        if count:
            raise _Refused(409, "min_artists", "Suis au moins un artiste pour continuer.")
        feed = pool.feed_state(request_app(), user)
        if feed["state"] != "unavailable":
            raise _Refused(
                409, "artists_available", "Des artistes sont disponibles : suis-en un."
            )
        _set_step(user, "done")
        return jsonify({"step": "done", "skipped": True, "liked": 0})
    if count < 1:
        raise _Refused(
            409, "min_artists", "Suis au moins un artiste pour continuer."
        )
    _set_step(user, "account")
    return jsonify({"step": "account"})


@onboarding_api.route("/onboarding/account/done", methods=["POST"])
@login_required
def onboarding_account_done():
    user = request.webuser
    step = _step(user)
    if step == "artists":
        raise _Refused(
            409, "artists_not_done", "Termine d'abord l'étape des artistes."
        )
    if step == "account":
        _set_step(user, "done")
    return jsonify({"step": "done", "user": {"onboarding": "done"}})


@onboarding_api.route("/onboarding/feed/retry", methods=["POST"])
@login_required
def onboarding_feed_retry():
    user = request.webuser
    instruction = _done_guard(user)
    if instruction is not None:
        raise instruction
    allowed, retry_after = retry_limiter.allow(f"u:{user.id}")
    if not allowed:
        raise _Refused(
            429,
            "rate_limited",
            "Une reconstruction vient déjà d'être demandée.",
            retry_after,
        )
    app = request_app()
    feed = pool.feed_state(app, user)
    if feed["ready"] and not feed["stale"]:
        return jsonify({"building": False, "ready": True})
    if feed["building"]:
        return jsonify({"building": True}), 202
    pool.ensure_feed(app, user, library_count=feed.get("library_count"), force=True)
    after = pool.feed_state(app, user)
    if after["ready"] and not after["building"]:
        return jsonify({"building": False, "ready": True})
    return jsonify({"building": True}), 202


@onboarding_api.route("/onboarding/picture/<entity_id>")
@login_required
def onboarding_picture(entity_id):
    try:
        size = int(request.args.get("s") or 250)
    except (TypeError, ValueError):
        size = 250
    found = pool.picture(request_app(), entity_id, size)
    if found is None:
        raise _not_found("Portrait introuvable.", "not_found")
    path, mimetype = found
    response = send_file(path, mimetype=mimetype, conditional=True)
    response.headers["Cache-Control"] = "public, max-age=86400, immutable"
    return response
