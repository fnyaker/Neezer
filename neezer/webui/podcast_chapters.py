# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""``GET /api/podcast/episode/<eid>/chapters`` — what the episode contains.

Seconds in the payload, milliseconds in the database. ``archived:false`` is a
200 (the player may ask about an episode that is about to archive); an unknown
episode is a 404. A first call on an archive that predates the feature fills
the derived cache under the extractor's own per-episode lock — it reads the
episode's file, never the audio stream, and never a provider.
"""

from __future__ import annotations

import logging
import os
import uuid

from flask import Blueprint, jsonify

from ..db import PodcastChapter, PodcastChapterScan, PodcastEpisode
from ..deezer import chapters

from . import login_required, same_origin_guard

logger = logging.getLogger(__name__)

chapters_api = Blueprint("chapters_api", __name__)

NOT_FOUND = {"error": "not found", "code": "not_found"}


@chapters_api.before_request
def reject_cross_site():
    return same_origin_guard()


@chapters_api.errorhandler(Exception)
def _json_error(exc):
    from werkzeug.exceptions import HTTPException

    if isinstance(exc, HTTPException):
        return jsonify({"error": exc.name}), exc.code
    logger.error("Unhandled error on %s", exc, exc_info=True)
    return jsonify({"error": "server error"}), 500


def _uuid_id(value):
    try:
        return uuid.UUID(str(value))
    except (ValueError, TypeError, AttributeError):
        return None


def _payload(episode_id, archived, scan, duration, rows):
    out = {
        "episode_id": str(episode_id),
        "archived": archived,
        "scanned": scan is not None,
        "source": scan[1] if scan is not None else None,
        "duration": int(duration or 0),
        "chapters": [],
    }
    if archived:
        for index, start_ms, end_ms, title in rows:
            out["chapters"].append(
                {
                    "index": index,
                    "start": int(start_ms) // 1000,
                    "end": (int(end_ms) // 1000) if end_ms is not None else None,
                    "title": title,
                }
            )
    return out


@chapters_api.route("/podcast/episode/<eid>/chapters")
@login_required
def episode_chapters(eid):
    episode_id = _uuid_id(eid)
    if episode_id is None:
        return jsonify(NOT_FOUND), 404

    row = (
        PodcastEpisode.select(
            PodcastEpisode.id,
            PodcastEpisode.path,
            PodcastEpisode.duration,
            PodcastChapterScan.scanned_at,
            PodcastChapterScan.source,
            PodcastChapterScan.n_chapters,
        )
        .left_outer_join(
            PodcastChapterScan, on=(PodcastChapterScan.episode == PodcastEpisode.id)
        )
        .where(PodcastEpisode.id == episode_id)
        .tuples()
        .first()
    )
    if row is None:
        return jsonify(NOT_FOUND), 404

    _id, path, duration, scanned_at, source, _count = row
    archived = bool(path and os.path.isfile(path))
    if not archived:
        return jsonify(_payload(_id, False, None, duration, ()))

    if scanned_at is None:
        episode = PodcastEpisode.get_or_none(PodcastEpisode.id == episode_id)
        if episode is None:
            return jsonify(NOT_FOUND), 404
        chapters.ensure(episode)
        scan = PodcastChapterScan.get_or_none(PodcastChapterScan.episode == episode_id)
        source = scan.source if scan is not None else None
        scanned_at = scan.scanned_at if scan is not None else None
    elif source == "none" and chapters.scan_is_stale(scanned_at):
        episode = PodcastEpisode.get_or_none(PodcastEpisode.id == episode_id)
        if episode is not None:
            chapters.enqueue_url(episode)

    scan = (scanned_at, source) if scanned_at is not None else None
    rows = (
        PodcastChapter.select(
            PodcastChapter.idx,
            PodcastChapter.start_ms,
            PodcastChapter.end_ms,
            PodcastChapter.title,
        )
        .where(PodcastChapter.episode == episode_id)
        .order_by(PodcastChapter.idx)
        .limit(chapters.MAX_CHAPTERS)
        .tuples()
    )
    return jsonify(_payload(_id, True, scan, duration, rows))
