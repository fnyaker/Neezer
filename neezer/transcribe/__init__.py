# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Speech-to-text of archived podcast episodes, on this server's own CPU.

The engine (``engine``) is plain whisper.cpp over ffmpeg; the sidecar
(``sidecar``) is the canonical text on the data disk; the queue (``job``) is the
index and the lease; the runner (``runner``) is the only place a subprocess is
started. Nothing here talks to a provider and nothing here needs Deezer.

The imports are deliberately lazy: the sidecar container and the real-engine
test import ``neezer.transcribe.engine`` in an image that has neither the
database nor Flask, and a package-wide ``import job`` would drag both in.
"""

__all__ = [
    "auto_enqueue",
    "drain",
    "enqueue",
    "estimate",
    "flush_sidecar",
    "queue_backlog",
    "rebuild",
    "resume_auto",
    "resume_if_interrupted",
    "start",
    "stop",
    "stop_all",
    "wake",
]


def auto_enqueue(episode, app=None):
    from .job import auto_enqueue as enqueue

    return enqueue(episode, app)


def enqueue(episode_id, user=None, force=False, language=None, app=None,
            source="server"):
    from .job import enqueue as put

    return put(episode_id, user=user, force=force, language=language, app=app,
               source=source)


def estimate(channel=None, app=None):
    from .job import estimate as run

    return run(channel=channel, app=app)


def flush_sidecar(episode_id, app=None):
    from .job import flush_sidecar as run

    return run(episode_id, app=app)


def queue_backlog(channel=None, backlog="all", user=None, app=None, limit=None):
    from .job import queue_backlog as run

    return run(channel=channel, backlog=backlog, user=user, app=app, limit=limit)


def rebuild(app=None, limit=None):
    from .job import rebuild as run

    return run(app=app, limit=limit)


def resume_auto(app=None):
    from .job import resume_auto as run

    return run(app)


def resume_if_interrupted(app=None):
    from .job import resume_if_interrupted as run

    return run(app)


def stop_all(app=None):
    from .job import stop_all as run

    return run(app)


def start(app=None):
    from .runner import start as run

    return run(app)


def stop():
    from .runner import stop as run

    return run()


def wake():
    from .runner import wake as run

    return run()


def drain(app=None, limit=None):
    from .runner import drain as run

    return run(app, limit=limit)
