# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Subsonic podcast endpoints, backed by Deezer shows/episodes.

Podcasts are materialized as ``PodcastChannel`` / ``PodcastEpisode`` rows (the
local DB is the source of truth for subscriptions). Episode audio is fetched and
archived on first stream/download exactly like Deezer tracks, then transcoded on
demand through the normal media pipeline.
"""

import logging
import os
import queue
import threading
import time

from flask import current_app, request

from ..db import PodcastChannel, PodcastEpisode

from . import get_entity, api_routing
from .exceptions import Forbidden, GenericError, MissingParameter, NotFound

logger = logging.getLogger(__name__)


def _provider():
    provider = getattr(current_app, "deezer", None)
    if provider is None:
        raise GenericError("Deezer proxy is not enabled")
    return provider


def _cfg():
    return current_app.config["DEEZER"]


def _podcast_owner():
    """User that owns imported podcast channels: the configured sync_user if any,
    else the requesting user."""
    from ..db import User

    sync_user = _cfg().get("sync_user")
    if sync_user:
        try:
            return User.get(name=sync_user)
        except User.DoesNotExist:
            pass
    return request.user


def _require_admin():
    if not request.user.admin:
        raise Forbidden()


@api_routing("/getPodcasts")
def get_podcasts():
    include_episodes = request.values.get("includeEpisodes", "true") != "false"
    pid = request.values.get("id")

    if pid is not None:
        channel = get_entity(PodcastChannel)
        channels = [channel]
    else:
        channels = list(PodcastChannel.select().order_by(PodcastChannel.title))

    return request.formatter(
        "podcasts",
        {
            "channel": [
                c.as_subsonic_channel(request.user, request.client, include_episodes)
                for c in channels
            ]
        },
    )


@api_routing("/getNewestPodcasts")
def get_newest_podcasts():
    count = min(int(request.values.get("count", 20) or 20), 100)
    episodes = (
        PodcastEpisode.select()
        .order_by(
            PodcastEpisode.publish_date.desc(), PodcastEpisode.created.desc()
        )
        .limit(count)
    )
    return request.formatter(
        "newestPodcasts",
        {
            "episode": [
                e.as_subsonic_episode(request.user, request.client) for e in episodes
            ]
        },
    )


@api_routing("/refreshPodcasts")
def refresh_podcasts():
    """Ask for a refresh of the subscribed shows, then answer at once.

    A refresh is one Deezer call per subscription, sequentially, under the
    shared rate limiter: at sixty shows that is seconds at best and a minute of
    timeouts when Deezer does not answer. The Subsonic client's button would
    hold one of the server's request threads for all of it. So the work moves to
    a single reniced worker; the response stays empty, which is the whole of the
    contract.
    """
    _require_admin()
    provider = _provider()
    app = current_app._get_current_object()
    started = start_refresh(
        app,
        provider,
        _podcast_owner().name,
        int(_cfg().get("podcast_episodes") or 30),
    )
    if not started:
        logger.info("Podcast refresh already running; not starting a second one")
    return request.formatter.empty


_refresh_lock = threading.Lock()
_refreshing = False
_refresh_thread = None


def refresh_thread():
    """The refresh worker while one runs (status reads, tests)."""
    with _refresh_lock:
        return _refresh_thread


def wait_refresh(timeout=30.0):
    """Wait until no refresh is running (status reads, tests)."""
    deadline = time.monotonic() + timeout
    while True:
        thread = refresh_thread()
        if thread is None:
            return True
        thread.join(max(0.0, deadline - time.monotonic()))
        if time.monotonic() >= deadline:
            return False


def start_refresh(app, provider, owner_name, episode_limit) -> bool:
    """Start the one refresh worker; False when one is already running."""
    global _refreshing, _refresh_thread
    with _refresh_lock:
        if _refreshing:
            return False
        _refreshing = True
        _refresh_thread = threading.Thread(
            target=_run_refresh,
            args=(app, provider, owner_name, episode_limit),
            name="podcast-refresh",
            daemon=True,
        )
        thread = _refresh_thread
    thread.start()
    return True


def _run_refresh(app, provider, owner_name, episode_limit):
    from ..db import close_connection, open_connection
    from ..deezer import backfill, workload
    from ..deezer.importer import DeezerImporter

    global _refreshing, _refresh_thread
    try:
        workload.renice(10)
        with app.app_context():
            open_connection(reuse=True)
            importer = DeezerImporter(provider, owner_name)
            synced = importer.sync_podcasts(int(episode_limit))
            # The new episodes of the shows whose switch is on go to the
            # archiver's background queue (the scheduled refresh does the same
            # through its sweep).
            for channel in PodcastChannel.select().where(PodcastChannel.auto_archive == True):  # noqa: E712
                backfill.archive_show(app, channel)
            logger.info("Podcast refresh done: %s show(s)", synced)
    except Exception as exc:
        logger.warning("Podcast refresh failed: %s", exc)
    finally:
        try:
            close_connection()
        except Exception:  # nosec B110 - the refresh thread is ending; the connection is dropped either way
            pass
        with _refresh_lock:
            _refreshing = False
            _refresh_thread = None


@api_routing("/createPodcastChannel")
def create_podcast_channel():
    _require_admin()
    url = request.values.get("url")
    if not url:
        raise MissingParameter("url")

    provider = _provider()
    from ..deezer.archive import parse_deezer_ref, import_show

    try:
        kind, did = parse_deezer_ref(url)
    except ValueError as e:
        raise GenericError(f"Unsupported podcast URL: {url}") from e

    if kind not in ("show", "episode"):
        raise GenericError("Only Deezer podcast (show) URLs are supported")

    show_id = did
    if kind == "episode":
        info = provider.dz.api.get_episode(did)
        show_id = (info.get("podcast") or {}).get("id") or info.get("podcast_id")
        if not show_id:
            raise GenericError(f"Could not resolve show for episode {did}")

    import_show(
        provider, _podcast_owner(), show_id,
        episode_limit=int(_cfg().get("podcast_episodes") or 30),
    )
    return request.formatter.empty


@api_routing("/deletePodcastChannel")
def delete_podcast_channel():
    _require_admin()
    channel = get_entity(PodcastChannel)

    # Never destroy archived audio: an episode already on disk is yours and
    # outlives both the subscription and the show. A channel with archived
    # episodes is kept and flagged unsubscribed; one with nothing on disk has
    # nothing to lose and is removed.
    if any(e.path and os.path.isfile(e.path) for e in channel.episodes):
        channel.subscribed = False
        channel.save()
        return request.formatter.empty
    channel.delete_instance(recursive=True)
    return request.formatter.empty


@api_routing("/deletePodcastEpisode")
def delete_podcast_episode():
    """Subsonic clients call this to free space. We don't: an archived episode
    is the only copy that survives the show leaving Deezer, so the file stays.
    Reported as done — the client's intent (stop showing it as downloaded) is
    not worth losing the audio over."""
    _require_admin()
    get_entity(PodcastEpisode)  # 404s on an unknown id, as before
    return request.formatter.empty


@api_routing("/downloadPodcastEpisode")
def download_podcast_episode():
    """Queue the episode on the local archiver, then answer at once.

    An episode is tens of megabytes from a third-party host: downloading it in
    the request held one of the server's threads for minutes, and a client that
    gave up and asked again started a second full download. The episode goes
    ``downloading`` then ``completed`` in ``getPodcasts``, which is what clients
    already read; the first *play* keeps its own synchronous stream
    (``/rest/stream``), because that one serves audio.
    """
    provider = _provider()
    episode = get_entity(PodcastEpisode)
    if episode.path and os.path.isfile(episode.path):
        return request.formatter.empty
    queued = queue_episode(provider, episode.id)
    if queued is None:
        raise GenericError("Too many episode downloads are already queued")
    return request.formatter.empty


#: Two at a time, like a person's own downloads: a slow podcast host must not
#: turn a client asking for a whole back catalogue into a thread per episode.
DOWNLOAD_WORKERS = 2
#: Every entry is a full episode on disk; past this the client is told so.
DOWNLOAD_QUEUE_MAX = 256

_download_lock = threading.Lock()
_download_queue = None
_download_workers = []
_download_pending = set()
_download_events = {}


def _workers():
    global _download_queue
    with _download_lock:
        if _download_workers:
            return
        _download_queue = queue.Queue(maxsize=DOWNLOAD_QUEUE_MAX)
        for i in range(DOWNLOAD_WORKERS):
            thread = threading.Thread(
                target=_download_worker,
                args=(_download_queue,),
                name=f"podcast-download-{i}",
                daemon=True,
            )
            thread.start()
            _download_workers.append(thread)


def queue_episode(provider, episode_id):
    """Ask for one episode to be archived. True queued, False already in flight,
    None when the queue is full (nothing was queued)."""
    _workers()
    key = str(episode_id)
    with _download_lock:
        if key in _download_pending:
            return False
        _download_pending.add(key)
        _download_events[key] = threading.Event()
    try:
        _download_queue.put_nowait((provider, key))
    except queue.Full:
        with _download_lock:
            _download_pending.discard(key)
            _download_events.pop(key, None)
        return None
    return True


def pending_episodes():
    """The episodes whose download is queued or running right now."""
    with _download_lock:
        return set(_download_pending)


def wait_downloads(timeout=30.0):
    """Wait until no episode download is in flight (status reads, tests)."""
    deadline = time.monotonic() + timeout
    while True:
        with _download_lock:
            events = list(_download_events.values())
            if not events:
                return True
        for event in events:
            event.wait(max(0.0, deadline - time.monotonic()))
        if time.monotonic() >= deadline:
            return False


def _download_worker(work):
    from ..db import PodcastEpisode, close_connection, open_connection
    from ..deezer import workload
    from ..deezer.archive import ensure_episode_archived

    workload.renice(10)
    while True:
        provider, key = work.get()
        try:
            open_connection(reuse=True)
            workload.quiet_wait(5.0)
            ensure_episode_archived(provider, PodcastEpisode[key])
        except Exception as exc:
            logger.warning("Podcast download failed for episode %s: %s", key, exc)
        finally:
            try:
                close_connection()
            except Exception:  # nosec B110 - the download worker already reported its own failure
                pass
            with _download_lock:
                _download_pending.discard(key)
                event = _download_events.pop(key, None)
            if event is not None:
                event.set()
            work.task_done()
