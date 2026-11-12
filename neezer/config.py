# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Copyright (C) 2013-2020 Alban 'spl0k' Féron
#                    2017 Óscar García Amor
#
# Distributed under terms of the GNU AGPLv3 license.

import os
import sys
import tempfile

from configparser import RawConfigParser

current_config = None


def get_current_config():
    return current_config or DefaultConfig()


def _default_data_dir():
    """``(dir, database file, socket file)`` for a config-less install.

    The pre-rebrand server kept its throwaway SQLite database at
    ``/tmp/supysonic/supysonic.db``. An upgraded host that never wrote a config
    must keep reading that file: pointing at a fresh ``/tmp/neezer/neezer.db``
    would look exactly like "all my users and playlists are gone". A configured
    ``database_uri`` always wins; this only covers the no-config case.
    """
    new = os.path.join(tempfile.gettempdir(), "neezer")
    old = os.path.join(tempfile.gettempdir(), "supysonic")
    if not os.path.isdir(new) and os.path.isfile(os.path.join(old, "supysonic.db")):
        return old, "supysonic.db", "supysonic.sock"
    return new, "neezer.db", "neezer.sock"


class DefaultConfig:
    DEBUG = False

    tempdir, _database_file, _socket_file = _default_data_dir()
    BASE = {
        "database_uri": "sqlite:///" + os.path.join(tempdir, _database_file),
        "scanner_extensions": None,
        "follow_symlinks": False,
    }
    WEBAPP = {
        "cache_dir": tempdir,
        "cache_size": 1024,
        "transcode_cache_size": 512,
        "log_file": None,
        "log_level": "WARNING",
        "log_rotate": True,
        "mount_webui": True,
        "mount_api": True,
        "mount_spa": True,
        "session_cookie_secure": False,
        # Maximum accepted request body size in MB (mainly bounds /api/upload —
        # without it any logged-in user could fill the disk with one request).
        # 0 disables the limit.
        "upload_max_size": 1024,
        # Per-user upload quota in GB for NON-admin users (admins upload without
        # limit). This is only the initial default: an admin can change it live
        # from the web UI, and that runtime value (stored in the DB) wins. 0
        # disables the per-user quota.
        "upload_quota_gb": 5,
        # Number of trusted reverse proxies in front of the app. 0 = none (the
        # app is reached directly); when > 0, X-Forwarded-* headers from that
        # many hops are trusted (ProxyFix). NEVER set this unless a proxy you
        # control really sits in front, or clients could spoof their IP/proto.
        "proxy_fix_hops": 0,
        "index_ignored_prefixes": "El La Le Las Les Los The",
        "online_lyrics": False,
        # Latest published Android app version (e.g. "1.4.0"). The web player
        # tells a native user to update when the app they run is older than
        # this. Empty = unknown, and then nothing is ever claimed. The container
        # fills it in from ANDROID_VERSION_NAME / APP_VERSION.
        "android_version": None,
        # Where that update is downloaded from (defaults to the project's
        # releases page).
        "android_url": None,
        # A local APK to serve as that update (a private or fork build). Empty =
        # the GitHub release of android_version on the repository android_url
        # points at.
        "android_apk": None,
    }
    DAEMON = {
        "socket": (
            r"\\.\pipe\neezer"
            if sys.platform == "win32"
            else os.path.join(tempdir, _socket_file)
        ),
        "run_watcher": True,
        "wait_delay": 5,
        "jukebox_command": None,
        "log_file": None,
        "log_level": "WARNING",
        "log_rotate": True,
    }
    LASTFM = {"api_key": None, "secret": None}
    LISTENBRAINZ = {"api_url": "https://api.listenbrainz.org"}
    # Completing facts from MusicBrainz (and AcoustID with a key) in the
    # background; see neezer/enrich/.
    ENRICH = {"enabled": True, "acoustid_key": None, "contact": None}
    TRANSCODING = {}
    MIMETYPES = {}
    DEEZER = {
        "enabled": False,
        "arl": None,
        "archive_dir": None,
        "default_quality": "FLAC",  # FLAC | MP3_320 | MP3_128
        "sync_user": None,  # neezer user owning imported playlists/favorites
        "sync_podcasts": True,  # refresh subscribed podcasts' episodes on each sync
        "podcast_episodes": 30,  # recent episodes to import per podcast
        # Import user-dropped audio files in archive_dir as local library tracks
        # (deezer_id NULL). They're searchable/playlistable like Deezer tracks but
        # the Deezer sync never touches them. Scanned on each sync run.
        "scan_local": True,
        "preload": True,
        "preload_count": 2,
        # Parallel workers for explicit "download now" batch pre-archiving
        # (whole album/playlist offline downloads). Capped at 8 in web.py.
        "download_workers": 4,
        # Auto-sync: on by default once a sync_user is set. Daily at sync_at
        # (default 04:00) unless sync_interval (minutes) is set, plus a run on
        # startup. The web app drives this; no cron/manual step needed.
        # After each sync, archive anything in the favorites / playlists /
        # subscribed podcasts that has no file yet, so the library survives
        # Deezer removing a track (or being unreachable). Adds only — it never
        # deletes an archived file. Off means audio is fetched on first play
        # only, as before.
        "archive_library": True,
        "sync_on_start": True,
        "sync_at": "04:00",  # daily auto-sync time "HH:MM"
        "sync_interval": 0,  # minutes; >0 overrides sync_at
    }

    def __init__(self):
        current_config = self


# The only settings a config object may hand to the Flask application. Flask's
# ``from_object`` copies EVERY uppercase attribute, so without this list a
# config file could define, say, a [secret_key] section and quietly replace the
# app's session-signing key (or any other Flask setting) from disk.
APP_CONFIG_KEYS = frozenset(
    {
        "DEBUG",
        "TESTING",
        "BASE",
        "WEBAPP",
        "DAEMON",
        "LASTFM",
        "LISTENBRAINZ",
        "ENRICH",
        "TRANSCODING",
        "MIMETYPES",
        "DEEZER",
    }
)


def app_config_from(config):
    """The subset of `config` that may be pushed into ``app.config``."""
    return {
        key: getattr(config, key) for key in APP_CONFIG_KEYS if hasattr(config, key)
    }


class IniConfig(DefaultConfig):
    # The baked/default system file comes first; a pre-rebrand file mounted at
    # /etc/supysonic then overrides it, and the user files follow (system then
    # user, old then new, current directory last). A deployment that upgrades
    # in place therefore keeps reading exactly the file it read before, and a
    # file written at the new path takes over once the old one is removed.
    common_paths = [
        "/etc/neezer",
        "/etc/supysonic",
        os.path.expanduser("~/.supysonic"),
        os.path.expanduser("~/.config/supysonic/supysonic.conf"),
        os.path.expanduser("~/.neezer"),
        os.path.expanduser("~/.config/neezer/neezer.conf"),
        "supysonic.conf",
        "neezer.conf",
    ]

    def __init__(self, paths):
        super().__init__()

        parser = RawConfigParser()
        parser.read(paths)

        for section in parser.sections():
            options = {k: self.__try_parse(v) for k, v in parser.items(section)}
            section = section.upper()

            if hasattr(self, section):
                # Copy before mutating: getattr(self, section) resolves to the
                # dict object defined on DefaultConfig, so updating it in place
                # edited CLASS state shared by every config instance in the
                # process (and leaked between them).
                merged = dict(getattr(self, section))
                merged.update(options)
                setattr(self, section, merged)
            else:
                setattr(self, section, options)

    @staticmethod
    def __try_parse(value):
        try:
            return int(value)
        except ValueError:
            try:
                return float(value)
            except ValueError:
                lv = value.lower()
                if lv in ("yes", "true", "on"):
                    return True
                if lv in ("no", "false", "off"):
                    return False
                return value

    @classmethod
    def from_common_locations(cls):
        # NEEZER_CONFIG (or its pre-rebrand alias SUPYSONIC_CONFIG) is read
        # LAST, so it overrides every discovered file. The container uses it to
        # point each process at a config rendered from ITS OWN environment: the
        # app and the daemon share the /data volume, so a single rendered path
        # meant whichever booted last overwrote the other's — and a container
        # with a partial environment silently dropped sections (the database
        # URI above all) for both of them.
        override = os.environ.get("NEEZER_CONFIG") or os.environ.get("SUPYSONIC_CONFIG")
        paths = list(cls.common_paths)
        if override:
            paths.append(override)
        return IniConfig(paths)
