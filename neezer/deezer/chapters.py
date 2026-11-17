# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Chapters an archived podcast episode carries in its own file.

Source precedence, once per episode: sidecar ``<archive>.chapters.json``, then
MP4 ``chpl``, then ID3 ``CHAP``/``CTOC``, then the publisher's chapters URL when
``dz.chapter_urls`` is ON (off by default), then nothing. Deezer is never a
source: its episode object has no chapter field.

No chapters means ``source="none"`` with a scan row (``scanned``), never an
invented ``Chapitre 1``. An explicit empty sidecar wins over the tags.

Storage is milliseconds (``podcast_chapter``); the API converts to seconds.
Only mutagen reads the files — ffprobe is not in the image. Nothing here writes
to an episode's audio, and a failure is never a verdict about the episode.
"""

from __future__ import annotations

import json
import logging
import math
import os
import re
import stat
import threading
from collections import deque
from dataclasses import dataclass
from urllib.parse import urljoin, urlsplit

import requests
from deezerpy import new_session
from requests.utils import get_environ_proxies, select_proxy

from ..db import (
    PodcastChapter,
    PodcastChapterScan,
    PodcastEpisode,
    db,
    now,
)
from .provider import (
    MAX_PINNED_ADDRESSES,
    _CAN_PIN_ADDRESS,
    _PinnedAddressAdapter,
    _warn_once_about_pinning,
    check_public_url,
    host_header,
)

logger = logging.getLogger(__name__)

MAX_CHAPTERS = 500
MAX_TITLE = 300
MAX_STARTS_MS = 24 * 3600 * 1000
MAX_SIDECAR_BYTES = 512 * 1024
MAX_ID3_TAG_BYTES = 10 * 1024 * 1024
MAX_JSON_BYTES = 256 * 1024

URL_TIMEOUT = (5, 10)
URL_REDIRECTS = 3
URL_RETRY_AFTER = 7 * 24 * 3600
URL_USER_AGENT = "Neezer (https://github.com/fnyaker/neezer)"

CURSOR_KEY = "pod.chapters"
PAGE_SIZE = 200
IN_CHUNK = 400
INSERT_CHUNK = 100

ID3_URL_DESCS = {"podcastchapters", "chapters", "chapter"}
_URL_FOLD = re.compile(r"[\s_\-:]+")
_WHITESPACE = re.compile(r"\s+")


@dataclass(frozen=True)
class Chapter:
    start_ms: int
    end_ms: "int | None"
    title: "str | None"
    source: str


def _title(raw) -> "str | None":
    if raw is None:
        return None
    text = _WHITESPACE.sub(" ", str(raw).replace("\x00", " ")).strip()
    text = "".join(c for c in text if c.isprintable())
    text = text[:MAX_TITLE].strip()
    return text or None


def _ms(value) -> "int | None":
    if isinstance(value, bool):
        return None
    if isinstance(value, str):
        try:
            value = float(value.strip())
        except ValueError:
            return None
    if not isinstance(value, (int, float)):
        return None
    if not math.isfinite(value):
        return None
    milliseconds = int(round(value * 1000))
    if milliseconds < 0 or milliseconds > MAX_STARTS_MS:
        return None
    return milliseconds


def _start(value) -> "int | None":
    if isinstance(value, str):
        return _ms(value)
    if not isinstance(value, (int, float)) or isinstance(value, bool):
        return None
    if not math.isfinite(value):
        return None
    milliseconds = int(value)
    if milliseconds < 0 or milliseconds > MAX_STARTS_MS:
        return None
    return milliseconds


def normalize(raw, *, duration_ms, source) -> "list[Chapter]":
    """Validate, order and bound chapter triples into ``Chapter`` objects."""
    items = []
    for entry in raw or ():
        try:
            start, end, title = entry
        except (TypeError, ValueError):
            continue
        start_ms = _start(start)
        if start_ms is None:
            continue
        end_ms = _start(end)
        if end_ms is not None and end_ms <= start_ms:
            end_ms = None
        if end_ms is not None and end_ms > start_ms + MAX_STARTS_MS:
            end_ms = None
        items.append((start_ms, end_ms, _title(title)))
    items.sort(key=lambda item: item[0])

    kept = []
    for item in items:
        if kept and item[0] - kept[-1][0] < 1000:
            continue
        kept.append(item)
    kept = kept[:MAX_CHAPTERS]

    duration_ms = int(duration_ms or 0)
    out = []
    for position, (start_ms, end_ms, title) in enumerate(kept):
        following = kept[position + 1][0] if position + 1 < len(kept) else None
        if following is not None:
            if end_ms is None or end_ms > following:
                end_ms = following
        elif end_ms is None:
            if duration_ms > start_ms:
                end_ms = duration_ms
        elif end_ms > duration_ms > start_ms:
            end_ms = duration_ms
        out.append(Chapter(start_ms=start_ms, end_ms=end_ms, title=title, source=source))
    return out


def parse_json_chapters(data) -> "list[tuple[int, int | None, str | None]]":
    """The Podcasting 2.0 ``chapters`` document (or a bare array) as triples.

    Raises ``ValueError`` on anything that is not that document, so a corrupt
    curation file is told apart from an explicit empty one.
    """
    if isinstance(data, bytes):
        try:
            data = data.decode("utf-8-sig")
        except UnicodeDecodeError as exc:
            raise ValueError("not UTF-8") from exc
    try:
        document = json.loads(data)
    except (TypeError, ValueError) as exc:
        raise ValueError("invalid JSON") from exc
    if isinstance(document, dict):
        raw = document.get("chapters")
    elif isinstance(document, list):
        raw = document
    else:
        raw = None
    if not isinstance(raw, list):
        raise ValueError("no chapters list")
    out = []
    for item in raw:
        if not isinstance(item, dict):
            continue
        if item.get("toc") is False:
            continue
        start = _ms(item.get("startTime"))
        if start is None:
            continue
        end = _ms(item.get("endTime"))
        out.append((start, end, item.get("title")))
    return out


def sidecar_path(path) -> str:
    base, _extension = os.path.splitext(str(path))
    return base + ".chapters.json"


def _archive_root(path) -> "str | None":
    try:
        return os.path.dirname(os.path.dirname(os.path.realpath(path)))
    except (OSError, ValueError):
        return None


def _read_sidecar(path) -> "tuple[list | None, str | None]":
    sidecar = sidecar_path(path)
    try:
        info = os.stat(sidecar)
    except OSError:
        return None, None
    if not stat.S_ISREG(info.st_mode):
        return None, "sidecar is not a regular file"
    if info.st_size > MAX_SIDECAR_BYTES:
        return None, "sidecar too large"
    try:
        real = os.path.realpath(sidecar)
    except OSError:
        return None, "sidecar unreadable"
    root = _archive_root(path)
    if root is None:
        return None, "sidecar outside the podcast archive"
    try:
        inside = os.path.commonpath([real, root]) == root
    except ValueError:
        inside = False
    if not inside:
        return None, "sidecar outside the podcast archive"
    try:
        with open(sidecar, "rb") as handle:
            data = handle.read(MAX_SIDECAR_BYTES + 1)
    except OSError:
        return None, "sidecar unreadable"
    if len(data) > MAX_SIDECAR_BYTES:
        return None, "sidecar too large"
    try:
        return parse_json_chapters(data), None
    except ValueError as exc:
        return None, f"invalid sidecar: {exc}"


def _cause(exc) -> str:
    return f"{type(exc).__name__}: {exc}"[:180]


def _id3_problem(path) -> "str | None":
    try:
        with open(path, "rb") as handle:
            head = handle.read(10)
    except OSError:
        return "unreadable file"
    if len(head) < 10 or head[:3] != b"ID3":
        return "no ID3 tag"
    size_bytes = head[6:10]
    if any(byte & 0x80 for byte in size_bytes):
        return "invalid ID3 tag size"
    size = (size_bytes[0] << 21) | (size_bytes[1] << 14) | (size_bytes[2] << 7) | size_bytes[3]
    if size > MAX_ID3_TAG_BYTES:
        return "ID3 tag too large"
    return None


def _open_id3(path):
    from mutagen.id3 import ID3

    problem = _id3_problem(path)
    if problem:
        raise ValueError(problem)
    return ID3(path)


def _mp4_chapters(path) -> "tuple[list, str | None]":
    from mutagen import MutagenError
    from mutagen.mp4 import MP4

    try:
        container = MP4(path)
    except MutagenError:
        return [], None
    except Exception as exc:
        return [], _cause(exc)
    try:
        listing = container.chapters
        if not listing:
            return [], None
        return [(int(round(chapter.start * 1000)), None, chapter.title) for chapter in listing], None
    except Exception as exc:
        return [], _cause(exc)


def _flatten_ctoc(tag, ctoc, depth, seen) -> list:
    out = []
    for child in ctoc.child_element_ids:
        if child in seen:
            continue
        seen.add(child)
        frame = tag.get(f"CHAP:{child}")
        if frame is not None:
            out.append(frame)
            continue
        if depth >= 3:
            continue
        nested = tag.get(f"CTOC:{child}")
        if nested is not None:
            out.extend(_flatten_ctoc(tag, nested, depth + 1, seen))
    return out


def _id3_chapters(path) -> "tuple[list, str | None]":
    from mutagen import MutagenError
    from mutagen.id3 import CTOCFlags

    try:
        tag = _open_id3(path)
    except MutagenError:
        return [], None
    except ValueError as exc:
        message = str(exc)
        return [], None if message == "no ID3 tag" else message
    except Exception as exc:
        return [], _cause(exc)

    try:
        frames = list(tag.getall("CHAP"))
        if not frames:
            return [], None
        ordered = []
        for ctoc in tag.getall("CTOC"):
            if ctoc.flags & CTOCFlags.TOP_LEVEL:
                ordered = _flatten_ctoc(tag, ctoc, 0, {ctoc.element_id})
                break
        if not ordered:
            ordered = frames
        out = []
        for frame in ordered:
            title = frame.sub_frames.get("TIT2")
            out.append((frame.start_time, frame.end_time, str(title) if title is not None else None))
        return out, None
    except Exception as exc:
        return [], _cause(exc)


def _embedded(path) -> "tuple[list, str, str | None]":
    chapters, error = _mp4_chapters(path)
    if chapters:
        return chapters, "mp4", None
    chapters, id3_error = _id3_chapters(path)
    if chapters:
        return chapters, "id3", None
    return [], "none", error or id3_error


def extract_embedded(path, *, duration_ms) -> "tuple[list[Chapter], str]":
    chapters, source, _error = _embedded(path)
    return normalize(chapters, duration_ms=duration_ms, source=source), source


def _fold(value) -> str:
    return _URL_FOLD.sub("", str(value or "").lower())


def _http_url(value) -> "str | None":
    text = str(value or "").strip().strip("\x00")
    if not text:
        return None
    parts = urlsplit(text)
    if parts.scheme not in ("http", "https") or not parts.hostname:
        return None
    return text


def url_from_tags(path) -> "str | None":
    """The publisher's chapters URL embedded in the file, if any."""
    try:
        found = list(_url_tags(path))
    except Exception:
        logger.debug("Could not read a chapters URL from %s", path, exc_info=True)
        return None
    for candidate in found:
        url = _http_url(candidate)
        if url:
            return url
    return None


def _url_tags(path):
    from mutagen import MutagenError
    from mutagen.mp4 import MP4

    try:
        tag = _open_id3(path)
    except (MutagenError, ValueError):
        tag = None
    except Exception:
        tag = None
    if tag is not None:
        for frame in tag.getall("TXXX"):
            if _fold(frame.desc) in ID3_URL_DESCS:
                yield from frame.text
        for frame in tag.getall("WXXX"):
            if _fold(frame.desc) in ID3_URL_DESCS:
                yield frame.url
        return
    try:
        container = MP4(path)
        tags = container.tags or {}
    except MutagenError:
        return
    except Exception:
        return
    for key, values in tags.items():
        if not str(key).startswith("----") or "chapter" not in str(key).lower():
            continue
        for value in values if isinstance(values, list) else [values]:
            if isinstance(value, bytes):
                yield value.decode("utf-8", "replace")
            else:
                yield value


def _fetch_once(url):
    addresses = check_public_url(url)
    parts = urlsplit(url)
    headers = {"User-Agent": URL_USER_AGENT, "Accept": "application/json"}

    if not _CAN_PIN_ADDRESS or select_proxy(url, get_environ_proxies(url)):
        _warn_once_about_pinning()
        session = new_session()
        return session, session.get(
            url, headers=headers, stream=True, timeout=URL_TIMEOUT, allow_redirects=False
        )

    headers["Host"] = host_header(parts)
    last = None
    for address in addresses[:MAX_PINNED_ADDRESSES]:
        session = new_session()
        adapter = _PinnedAddressAdapter(address)
        session.mount("https://", adapter)
        session.mount("http://", adapter)
        try:
            return session, session.get(
                url, headers=headers, stream=True, timeout=URL_TIMEOUT, allow_redirects=False
            )
        except requests.RequestException as exc:
            session.close()
            last = exc
    raise requests.RequestException(
        f"could not reach {parts.hostname}: {last}"
    ) from last


def fetch_json_chapters(url) -> "tuple[list | None, str | None]":
    """GET a publisher's chapters document through the SSRF guard.

    Every redirect hop is validated and pinned again, no cookie survives a hop,
    the body is capped at ``MAX_JSON_BYTES`` and must be JSON.
    """
    current = url
    for _hop in range(URL_REDIRECTS + 1):
        try:
            session, response = _fetch_once(current)
        except Exception as exc:
            return None, _cause(exc)
        try:
            with response as resp:
                if resp.is_redirect or resp.is_permanent_redirect:
                    location = resp.headers.get("Location")
                    if not location:
                        return None, "redirect without a Location"
                    current = urljoin(current, location)
                    continue
                resp.raise_for_status()
                content_type = (resp.headers.get("Content-Type") or "").split(";")[0].strip().lower()
                if "html" in content_type or "xml" in content_type:
                    return None, f"not JSON ({content_type})"
                body = bytearray()
                for chunk in resp.iter_content(8192):
                    if chunk:
                        body += chunk
                        if len(body) > MAX_JSON_BYTES:
                            return None, "chapters document too large"
                if body.lstrip()[:1] not in (b"{", b"["):
                    return None, "not JSON"
                try:
                    return parse_json_chapters(bytes(body)), None
                except ValueError as exc:
                    return None, str(exc)
        except requests.RequestException as exc:
            return None, _cause(exc)
        finally:
            session.close()
    return None, "too many redirects"


def extract(path, *, duration_ms, allow_url) -> "tuple[list[Chapter], str, str | None]":
    """Everything one episode's own file can say about its chapters."""
    error = None
    sidecar, sidecar_error = _read_sidecar(path)
    if sidecar is not None:
        chapters = normalize(sidecar, duration_ms=duration_ms, source="sidecar")
        return chapters, ("sidecar" if chapters else "none"), sidecar_error
    if sidecar_error:
        error = sidecar_error

    raw, source, embedded_error = _embedded(path)
    if raw:
        return normalize(raw, duration_ms=duration_ms, source=source), source, error or embedded_error
    if embedded_error:
        error = error or embedded_error

    if allow_url:
        url = url_from_tags(path)
        if url:
            document, fetch_error = fetch_json_chapters(url)
            if document is not None:
                chapters = normalize(document, duration_ms=duration_ms, source="json")
                return chapters, ("json" if chapters else "none"), fetch_error
            error = error or fetch_error
    return [], "none", error


def _app():
    from flask import current_app, has_app_context

    return current_app._get_current_object() if has_app_context() else None


def _rule(name, default=False) -> bool:
    from . import rules

    try:
        return bool(rules.load(_app()).get(name, default))
    except Exception:
        logger.debug("Could not read the chapter archive rules", exc_info=True)
        return default


def enabled() -> bool:
    return _rule("chapters", True)


def _urls_enabled() -> bool:
    return _rule("chapter_urls", False)


_locks = {}
_locks_guard = threading.Lock()


def _lock_for(episode_id):
    key = str(episode_id)
    with _locks_guard:
        lock = _locks.get(key)
        if lock is None:
            if len(_locks) > 512:
                _locks.clear()
            lock = _locks[key] = threading.Lock()
        return lock


def _store(episode_id, chapters, source, error) -> None:
    from peewee import IntegrityError

    rows = [
        {
            "episode": episode_id,
            "idx": index,
            "start_ms": chapter.start_ms,
            "end_ms": chapter.end_ms,
            "title": chapter.title,
            "source": chapter.source,
        }
        for index, chapter in enumerate(chapters[:MAX_CHAPTERS])
    ]
    cause = str(error)[:200] if error else None
    try:
        with db.atomic():
            PodcastChapter.delete().where(PodcastChapter.episode == episode_id).execute()
            for start in range(0, len(rows), INSERT_CHUNK):
                PodcastChapter.insert_many(rows[start : start + INSERT_CHUNK]).execute()
            PodcastChapterScan.delete().where(
                PodcastChapterScan.episode == episode_id
            ).execute()
            PodcastChapterScan.create(
                episode=episode_id,
                scanned_at=now(),
                source=source,
                n_chapters=len(rows),
                error=cause,
            )
    except IntegrityError:
        logger.debug("Chapter scan of %s raced another writer", episode_id, exc_info=True)


def ensure(episode, *, force=False, allow_url=False) -> int:
    """Extract and store one episode's chapters, best effort. Returns the count.

    Never raises for a malformed file, and writes nothing when the episode has
    no readable archive: an unarchived episode is rescanned once it is. The
    ``chapters`` switch silences the automatic callers; ``force`` is an
    explicit command and goes through.
    """
    if not force and not enabled():
        return 0
    path = getattr(episode, "path", None)
    if not path or not os.path.isfile(path):
        return 0
    episode_id = getattr(episode, "id", None)
    if episode_id is None:
        return 0
    with _lock_for(episode_id):
        scan = PodcastChapterScan.get_or_none(PodcastChapterScan.episode == episode_id)
        if scan is not None and not force:
            if not (allow_url and scan.source == "none"):
                return scan.n_chapters
        duration_ms = int(getattr(episode, "duration", 0) or 0) * 1000
        try:
            chapters, source, error = extract(
                path, duration_ms=duration_ms, allow_url=allow_url
            )
        except Exception as exc:
            logger.debug("Chapter extraction failed for %s", path, exc_info=True)
            chapters, source, error = [], "none", _cause(exc)
        if len(chapters) >= MAX_CHAPTERS:
            logger.warning(
                "chapter cap reached for episode %s (%d chapters kept)", episode_id, len(chapters)
            )
        _store(episode_id, chapters, source, error)
        if source == "none" and not allow_url:
            enqueue_url(episode)
        return len(chapters)


def counts_for(episode_ids) -> dict:
    """``{episode id: number of chapters}`` in one grouped query per 400 ids."""
    from peewee import fn

    out = {}
    ids = list(episode_ids)
    for start in range(0, len(ids), IN_CHUNK):
        rows = (
            PodcastChapter.select(PodcastChapter.episode, fn.COUNT(PodcastChapter.id))
            .where(PodcastChapter.episode << ids[start : start + IN_CHUNK])
            .group_by(PodcastChapter.episode)
            .tuples()
        )
        for episode_id, count in rows:
            out[episode_id] = count
    return out


_queue = deque()
_queue_lock = threading.Lock()
_queue_thread = None


def _testing() -> bool:
    app = _app()
    return bool(app is not None and getattr(app, "testing", False))


def enqueue_url(episode) -> bool:
    """Queue one BULK fetch of the publisher's chapters URL, if there is one."""
    global _queue_thread

    if not _urls_enabled():
        return False
    path = getattr(episode, "path", None)
    episode_id = getattr(episode, "id", None)
    if not path or episode_id is None or not os.path.isfile(path):
        return False
    if url_from_tags(path) is None:
        return False
    with _queue_lock:
        if episode_id in _queue:
            return True
        _queue.append(episode_id)
        if _testing():
            return True
        if _queue_thread is None or not _queue_thread.is_alive():
            _queue_thread = threading.Thread(
                target=_queue_worker, name="podcast-chapter-urls", daemon=True
            )
            _queue_thread.start()
    return True


def _queue_worker() -> None:
    global _queue_thread

    from .workload import quiet_wait, renice

    renice(15)
    try:
        db.connect(reuse_if_open=True)
    except Exception:
        logger.debug("Episode queue could not open the database", exc_info=True)
    while True:
        with _queue_lock:
            if not _queue:
                _queue_thread = None
                return
            episode_id = _queue.popleft()
        quiet_wait(2.0)
        try:
            fetch_for_episode(episode_id)
        except Exception as exc:
            logger.info("Chapter URL fetch failed for %s: %s", episode_id, exc)


def drain_urls() -> int:
    """Run the queued URL fetches on this thread (tests only)."""
    done = 0
    while True:
        with _queue_lock:
            if not _queue:
                return done
            episode_id = _queue.popleft()
        done += 1
        try:
            fetch_for_episode(episode_id)
        except Exception as exc:
            logger.info("Chapter URL fetch failed for %s: %s", episode_id, exc)


def fetch_for_episode(episode_id) -> int:
    """Fetch the publisher's chapters document and replace the episode's rows."""
    if not _urls_enabled():
        return 0
    episode = PodcastEpisode.get_or_none(PodcastEpisode.id == episode_id)
    if episode is None:
        return 0
    path = episode.path
    if not path or not os.path.isfile(path):
        return 0
    url = url_from_tags(path)
    if url is None:
        return 0
    document, error = fetch_json_chapters(url)
    duration_ms = int(episode.duration or 0) * 1000
    if document is None:
        chapters, source = [], "none"
    else:
        chapters = normalize(document, duration_ms=duration_ms, source="json")
        source = "json" if chapters else "none"
    with _lock_for(episode_id):
        _store(episode_id, chapters, source, error)
    return len(chapters)


def scan_is_stale(scanned_at) -> bool:
    if scanned_at is None:
        return True
    try:
        return (now() - scanned_at).total_seconds() > URL_RETRY_AFTER
    except (TypeError, AttributeError):
        return True


def make_job(force=False, allow_url=False, page_size=PAGE_SIZE):
    """A resumable walk over archived episodes, one header read per episode."""
    from ..libjob import LibraryJob

    scanned = PodcastChapterScan.select(PodcastChapterScan.episode)
    query = PodcastEpisode.select(
        PodcastEpisode.id, PodcastEpisode.path, PodcastEpisode.duration
    ).where(PodcastEpisode.path.is_null(False))
    if not force:
        query = query.where(~PodcastEpisode.id.in_(scanned))

    def work(episode):
        path = episode.path
        if not path or not os.path.isfile(path):
            return "archive missing"
        ensure(episode, force=True, allow_url=allow_url)
        return None

    return LibraryJob(
        "podcast chapters",
        query,
        work,
        cursor_key=CURSOR_KEY,
        force=force,
        page_size=page_size,
    )


def _pending() -> bool:
    scanned = PodcastChapterScan.select(PodcastChapterScan.episode)
    return (
        PodcastEpisode.select()
        .where(PodcastEpisode.path.is_null(False) & ~PodcastEpisode.id.in_(scanned))
        .exists()
    )


def start_backfill(app):
    """One-shot, resumable scan of archives that predate this feature."""
    try:
        if not enabled():
            return None
        if not _pending():
            return None
    except Exception:
        logger.debug("Could not plan the podcast chapter backfill", exc_info=True)
        return None
    job = make_job()
    if getattr(app, "testing", False):
        return job

    def target():
        with app.app_context():
            job.run()

    threading.Thread(target=target, name="job-podcast-chapters", daemon=True).start()
    return job
