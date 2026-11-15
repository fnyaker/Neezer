# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""The Android app's own update, served by this server.

The app used to send the user to a GitHub page: leave the app, find the asset,
download it in a browser, open it from the notifications. Now the app asks THIS
server for the APK (``GET /api/android/apk``), checks it is a newer build of
itself signed with its own key, and hands it to Android's installer — one tap.

Where the file comes from, best first:

- ``[webapp] android_apk``: a local file the operator put there (a private or
  fork build). Served as is.
- the GitHub release of the declared ``android_version``, on the repository
  ``android_url`` points at (the project's by default). Fetched ONCE, by a
  single worker: the first request for that version answers ``202`` with a
  ``Retry-After``, the following ones serve the file as soon as it is on disk.
  A 10 MB download never happens in a request thread, and never behind a lock
  another device is waiting on.

Nothing here is a trust decision: Android refuses to install an update that is
not signed with the installed app's certificate, and the app checks that itself
before asking (so it can say why rather than failing in the installer). The
server only has to never be a way to fetch anything but that one file.
"""

from __future__ import annotations

import logging
import os
import re
import threading
import time
from urllib.parse import urlsplit

import requests
from flask import current_app, jsonify, send_file

from . import login_required, webapi

logger = logging.getLogger(__name__)

APK_NAME = "neezer.apk"
# The asset name published before the rebrand. A release that predates the
# rename still carries only this one, and the server must keep offering it.
LEGACY_APK_NAME = "nsupysonic.apk"
APK_MIME = "application/vnd.android.package-archive"
# A release APK is ~10 MB; anything past this is not one.
MAX_APK_BYTES = 200 * 1024 * 1024
# Connect, then between two reads: a stalled GitHub must not hold the thread.
TIMEOUT = (10, 60)
# What a tag may look like before it goes into a URL path.
_VERSION_RE = re.compile(r"\Av?[0-9A-Za-z][0-9A-Za-z._-]{0,39}\Z")
# github.com/<owner>/<repo>/...: the repository a release link points at.
_REPO_RE = re.compile(r"\A/([A-Za-z0-9_.-]{1,100})/([A-Za-z0-9_.-]{1,100})(?:/|\Z)")
# Hosts a release asset download may land on (GitHub redirects to its CDN).
_ASSET_HOSTS = ("github.com", "objects.githubusercontent.com",
                "release-assets.githubusercontent.com")
DEFAULT_REPO = ("fnyaker", "Neezer")


def declared_version(cfg: dict) -> str | None:
    version = (cfg.get("android_version") or "").strip()
    return version if _VERSION_RE.match(version) else None


def _repo_of(url: str | None) -> tuple[str, str] | None:
    if not url:
        return DEFAULT_REPO
    parts = urlsplit(url)
    if parts.scheme != "https" or parts.hostname != "github.com":
        return None
    m = _REPO_RE.match(parts.path)
    return (m.group(1), m.group(2)) if m else None


def apk_source(cfg: dict) -> tuple[str, str] | None:
    """``("file", path)``, ``("github", url)`` or None when there is no APK to offer."""
    local = (cfg.get("android_apk") or "").strip()
    if local:
        return ("file", local) if os.path.isfile(local) else None
    version = declared_version(cfg)
    repo = _repo_of((cfg.get("android_url") or "").strip() or None)
    if not version or not repo:
        return None
    owner, name = repo
    return (
        "github",
        f"https://github.com/{owner}/{name}/releases/download/{version}/{APK_NAME}",
    )


def _cache_path(cfg: dict, version: str) -> str:
    return os.path.join(cfg["cache_dir"], "android", f"neezer-{version}.apk")


def _download(url: str, dest: str) -> None:
    """Fetch the release asset into ``dest``; raises on anything but an APK."""
    os.makedirs(os.path.dirname(dest), exist_ok=True)
    part = dest + ".part"
    try:
        with requests.get(url, stream=True, timeout=TIMEOUT) as rv:
            final = urlsplit(rv.url)
            if final.scheme != "https" or final.hostname not in _ASSET_HOSTS:
                raise OSError(f"redirected off GitHub ({final.hostname})")
            rv.raise_for_status()
            size = 0
            with open(part, "wb") as fh:
                for chunk in rv.iter_content(64 * 1024):
                    size += len(chunk)
                    if size > MAX_APK_BYTES:
                        raise OSError("larger than any APK")
                    fh.write(chunk)
        with open(part, "rb") as fh:
            # An APK is a zip; a GitHub error page is not.
            if fh.read(4) != b"PK\x03\x04":
                raise OSError("not an APK")
        os.replace(part, dest)
    finally:
        if os.path.exists(part):
            os.remove(part)
    # Older versions are never served again. Both the current and the
    # pre-rebrand cache prefix are cleared, so an upgraded server does not keep
    # a stale 10 MB APK next to the new one.
    folder = os.path.dirname(dest)
    for entry in os.listdir(folder):
        path = os.path.join(folder, entry)
        if path != dest and entry.endswith(".apk") and entry.startswith(
            ("neezer-", "nsupysonic-")
        ):
            try:
                os.remove(path)
            except OSError:
                pass


def _legacy_url(url: str) -> str | None:
    """The same release asset under its pre-rebrand name, if this is that URL."""
    tail = "/" + APK_NAME
    return url[: -len(tail)] + "/" + LEGACY_APK_NAME if url.endswith(tail) else None


def fetch_apk(cfg: dict) -> str:
    """Download the declared release into the cache, blocking.

    The worker's whole job (the route never calls it): a slow GitHub used to
    hold the request thread — and the global lock behind it, so every other
    device waited too — for the length of a 10 MB download.
    """
    source = apk_source(cfg)
    if source is None or source[0] != "github":
        raise OSError("no release to fetch")
    return _fetch_asset(source[1], _cache_path(cfg, declared_version(cfg)))


def _fetch_asset(where: str, dest: str) -> str:
    os.makedirs(os.path.dirname(dest), exist_ok=True)
    if os.path.isfile(dest):
        return dest
    try:
        _download(where, dest)
    except requests.HTTPError as exc:
        # Only a missing asset (404) is worth the pre-rebrand name: a
        # corrupt download or a redirect off GitHub is a real failure.
        status = getattr(getattr(exc, "response", None), "status_code", None)
        legacy = _legacy_url(where)
        if status != 404 or legacy is None:
            raise
        _download(legacy, dest)
    return dest


#: How long a failed fetch is remembered before a tap may start a new one.
APK_RETRY = 30.0
#: What a client is told to wait while a fetch runs: an APK is ~10 MB, and the
#: app asks again rather than holding its own thread on a socket.
APK_WAIT = 5

_apk_lock = threading.Lock()
_apk_jobs = {}  # dest -> {"state": "fetching"|"ready"|"failed", "error", "at", "thread"}


def cached_apk(cfg: dict) -> str | None:
    """The file to serve right now, without asking anyone."""
    source = apk_source(cfg)
    if source is None:
        return None
    kind, where = source
    if kind == "file":
        return where
    dest = _cache_path(cfg, declared_version(cfg))
    return dest if os.path.isfile(dest) else None


def apk_job(cfg: dict) -> dict | None:
    """The last fetch of the declared version: its state, its cause, its thread."""
    version = declared_version(cfg)
    if version is None or not cfg.get("cache_dir"):
        return None
    with _apk_lock:
        job = _apk_jobs.get(_cache_path(cfg, version))
        return dict(job) if job is not None else None


def ensure_apk(cfg: dict) -> str | None:
    """The path to serve, or None after making sure ONE worker fetches it.

    Two devices asking at once share one download; a third one arriving while
    it runs is told to come back rather than waiting on it.
    """
    path = cached_apk(cfg)
    if path is not None:
        return path
    source = apk_source(cfg)
    if source is None or source[0] != "github":
        return None
    dest = _cache_path(cfg, declared_version(cfg))
    where = source[1]
    with _apk_lock:
        job = _apk_jobs.get(dest)
        if job is not None:
            if job["state"] == "fetching":
                return None
            if time.monotonic() - job["at"] < APK_RETRY:
                return None
        job = {"state": "fetching", "error": None, "at": time.monotonic(), "thread": None}
        job["thread"] = threading.Thread(
            target=_fetch, args=(where, dest), name="android-apk", daemon=True
        )
        _apk_jobs[dest] = job
    job["thread"].start()
    return None


def _fetch(where: str, dest: str) -> None:
    from ..deezer import workload

    workload.renice(10)
    error = None
    try:
        _fetch_asset(where, dest)
    except (OSError, requests.RequestException) as exc:
        error = exc
        logger.warning("Android update unavailable: %s", exc)
    except Exception as exc:
        error = exc
        logger.warning("Android update fetch crashed: %s", exc)
    with _apk_lock:
        job = _apk_jobs.get(dest)
        if job is not None:
            job["error"] = None if error is None else type(error).__name__
            job["state"] = "ready" if os.path.isfile(dest) else "failed"
            job["at"] = time.monotonic()


@webapi.route("/android/apk")
@login_required
def android_apk():
    cfg = current_app.config["WEBAPP"]
    version = declared_version(cfg)
    path = ensure_apk(cfg)
    if path is None:
        if apk_source(cfg) is None:
            return jsonify({"error": "Aucune mise à jour Android publiée"}), 404
        job = apk_job(cfg) or {}
        failed = job.get("state") == "failed"
        retry = (
            max(1, int(APK_RETRY - (time.monotonic() - job.get("at", 0))) + 1)
            if failed
            else APK_WAIT
        )
        headers = {"Retry-After": str(retry)}
        if version:
            headers["X-NS-Version"] = version
        if failed:
            return (
                jsonify({"state": "failed", "error": "Impossible de récupérer la mise à jour"}),
                502,
                headers,
            )
        return jsonify({"state": "fetching"}), 202, headers
    response = send_file(
        path, mimetype=APK_MIME, as_attachment=True, download_name=APK_NAME,
        conditional=True,
    )
    if version:
        response.headers["X-NS-Version"] = version
    return response
