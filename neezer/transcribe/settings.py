# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""What the transcription engine may use, read from the configuration file.

Paths stay file-only (``bin``, ``model``): no request may name one. The other
values have safe defaults so a server without whisper.cpp boots with the feature
off and a named reason, never an exception. Runtime overrides in ``Meta`` are a
later lot; this module only merges the ``[transcribe]`` section over the
defaults and validates every value.
"""

from __future__ import annotations

import os
import shutil

from . import engine, sidecar

DEFAULTS = {
    "enabled": False,
    "bin": None,
    "model": None,
    "runner": "inline",
    "daily_minutes": 120,
    "search_segments": 1_200_000,
    "max_minutes": 360,
    "language": "auto",
}

RUNNERS = ("inline", "external")
BACKLOG_MODES = ("all", "recent", "none")
BACKLOG_DEFAULT = "all"
MAX_DAILY_MINUTES = 1440
MAX_LIMIT_MINUTES = 24 * 60
MAX_SEARCH_SEGMENTS = 10_000_000


def _sections(app=None):
    if app is not None:
        config = getattr(app, "config", None)
        if config is not None:
            return (
                dict(config.get("TRANSCRIBE") or {}),
                dict(config.get("DEEZER") or {}),
                dict(config.get("WEBAPP") or {}),
            )
    from ..config import get_current_config

    config = get_current_config()
    return (
        dict(getattr(config, "TRANSCRIBE", None) or {}),
        dict(getattr(config, "DEEZER", None) or {}),
        dict(getattr(config, "WEBAPP", None) or {}),
    )


def _as_bool(value, default):
    if isinstance(value, bool):
        return value
    if isinstance(value, str):
        lowered = value.strip().lower()
        if lowered in ("1", "true", "yes", "on"):
            return True
        if lowered in ("0", "false", "no", "off", ""):
            return False
    if isinstance(value, (int, float)):
        return bool(value)
    return default


def _as_int(value, default, low, high):
    try:
        number = int(value)
    except (TypeError, ValueError):
        return default
    return max(low, min(high, number))


def _as_text(value):
    if value is None:
        return None
    text = str(value).strip()
    return text or None


def load(app=None):
    values = dict(DEFAULTS)
    values.update(_sections(app)[0])
    runner = str(values.get("runner") or "inline").strip().lower()
    values["enabled"] = _as_bool(values.get("enabled"), DEFAULTS["enabled"])
    values["runner"] = runner if runner in RUNNERS else "inline"
    values["bin"] = _as_text(values.get("bin"))
    values["model"] = _as_text(values.get("model"))
    values["daily_minutes"] = _as_int(
        values.get("daily_minutes"), DEFAULTS["daily_minutes"], 0, MAX_DAILY_MINUTES
    )
    values["search_segments"] = _as_int(
        values.get("search_segments"),
        DEFAULTS["search_segments"],
        0,
        MAX_SEARCH_SEGMENTS,
    )
    values["max_minutes"] = _as_int(
        values.get("max_minutes"), DEFAULTS["max_minutes"], 5, MAX_LIMIT_MINUTES
    )
    values["language"] = (
        engine.normalize_language(values.get("language"))
        or DEFAULTS["language"]
    )
    return values


def paths(app=None):
    _transcribe, deezer, webapp = _sections(app)
    values = load(app)
    archive = _as_text(deezer.get("archive_dir"))
    cache = _as_text(webapp.get("cache_dir"))
    ffmpeg = shutil.which("ffmpeg") or "ffmpeg"
    return {
        "bin": values["bin"],
        "model": values["model"],
        "ffmpeg": ffmpeg,
        "archive": archive,
        "cache": cache,
        "root": sidecar.root_for(archive),
    }


def archive_root(app=None):
    return paths(app)["root"]


def cache_dir(app=None):
    values = paths(app)
    return values["cache"] or os.path.join(
        os.path.expanduser("~"), ".cache", "neezer"
    )


def enabled(app=None):
    return load(app)["enabled"]


def runner(app=None):
    return load(app)["runner"]


def language(app=None):
    return load(app)["language"]


def max_minutes(app=None):
    return load(app)["max_minutes"]


def daily_minutes(app=None):
    return load(app)["daily_minutes"]


def search_segments(app=None):
    return load(app)["search_segments"]


def threads():
    return engine.threads()


def probe(app=None):
    return engine.probe(paths(app))


def invalidate():
    engine.clear_probe_cache()


def available(app=None):
    values = load(app)
    if not values["enabled"]:
        return False, "disabled"
    if values["runner"] == "external":
        return True, None
    verdict = engine.quick_check(paths(app))
    return bool(verdict["ok"]), verdict["reason"]
