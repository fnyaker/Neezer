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
  fork build). Served once ``_apk_problem`` says it is an APK.
- the GitHub release of the declared ``android_version``, on the repository
  ``android_url`` points at (the project's by default). Fetched ONCE, by a
  single worker: the first request for that version answers ``202`` with a
  ``Retry-After``, the following ones serve the file as soon as it is on disk.
  A 10 MB download never happens in a request thread, and never behind a lock
  another device is waiting on.

Every byte this module hands out has been checked first: a body is read whole,
its length compared with what the response announced, and the file must open as
a zip carrying Android's binary ``AndroidManifest.xml`` — on the way in (every
member's CRC read) and again on every serve (the central directory only). A
cached file that fails that check is deleted and fetched again, instead of being
served to the app for ever.

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
import zipfile
from urllib.parse import urlsplit

import requests
from flask import current_app, jsonify, send_file

from . import login_required, webapi

logger = logging.getLogger(__name__)

APK_NAME = "neezer.apk"
APK_MIME = "application/vnd.android.package-archive"
# A release APK is ~10 MB; anything past this is not one.
MAX_APK_BYTES = 200 * 1024 * 1024
# And anything below this is not one either (a real build is a thousand times bigger).
MIN_APK_BYTES = 64 * 1024
# What Android's BINARY XML manifest starts with (RES_XML_TYPE, 8-byte header).
ANDROID_MANIFEST_HEADER = b"\x03\x00\x08\x00"
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


def _apk_problem(path: str, deep: bool = False) -> str | None:
    """Why ``path`` is not an installable APK, or None when it looks like one.

    ``deep`` reads every member (the zip's own CRCs) and the first bytes of the
    manifest, which only Android's binary XML starts with: that is what a
    finished download pays before it is published. Serving a cached file only
    opens the central directory, so the check costs milliseconds.
    """
    try:
        size = os.path.getsize(path)
    except OSError:
        return "absent"
    if size < MIN_APK_BYTES:
        return f"{size} octets, plus court qu'un APK"
    if size > MAX_APK_BYTES:
        return f"{size} octets, au-delà du plafond"
    try:
        with zipfile.ZipFile(path) as archive:
            if "AndroidManifest.xml" not in archive.namelist():
                return "aucun AndroidManifest.xml"
            with archive.open("AndroidManifest.xml") as manifest:
                head = manifest.read(len(ANDROID_MANIFEST_HEADER))
            if head != ANDROID_MANIFEST_HEADER:
                return "AndroidManifest.xml n'est pas du XML binaire Android"
            if deep and archive.testzip() is not None:
                return "un membre ne correspond pas à sa somme de contrôle"
    except (zipfile.BadZipFile, zipfile.LargeZipFile, OSError, ValueError):
        return "zip illisible"
    return None


def _discard(path: str, why: str) -> None:
    logger.warning("Mise à jour Android écartée (%s) : %s", why, path)
    try:
        os.remove(path)
    except OSError:
        pass


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
            announced = rv.headers.get("Content-Length")
            size = 0
            with open(part, "wb") as fh:
                for chunk in rv.iter_content(64 * 1024):
                    size += len(chunk)
                    if size > MAX_APK_BYTES:
                        raise OSError("larger than any APK")
                    fh.write(chunk)
        if announced is not None:
            try:
                announced_size = int(announced)
            except (TypeError, ValueError):
                announced_size = None
            if announced_size is not None and announced_size != size:
                raise OSError(f"tronqué ({size} octets sur {announced_size})")
        problem = _apk_problem(part, deep=True)
        if problem is not None:
            raise OSError(f"not an APK: {problem}")
        os.replace(part, dest)
    finally:
        if os.path.exists(part):
            os.remove(part)
    # Older versions are never served again. The pre-rebrand cache prefix is
    # cleared too, so an upgraded server does not keep 10 MB nobody serves.
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
        problem = _apk_problem(dest)
        if problem is None:
            return dest
        _discard(dest, problem)
    _download(where, dest)
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
        problem = _apk_problem(where)
        if problem is None:
            return where
        logger.warning("Fichier de mise à jour Android inutilisable (%s) : %s", problem, where)
        return None
    dest = _cache_path(cfg, declared_version(cfg))
    if not os.path.isfile(dest):
        return None
    problem = _apk_problem(dest)
    if problem is None:
        return dest
    _discard(dest, problem)
    return None


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
    source = apk_source(cfg)
    path = ensure_apk(cfg)
    if path is None:
        if source is None:
            return jsonify({"error": "Aucune mise à jour Android publiée"}), 404
        headers = {"Retry-After": str(APK_RETRY)}
        if version:
            headers["X-NS-Version"] = version
        if source[0] == "file":
            return (
                jsonify(
                    {
                        "state": "failed",
                        "error": (
                            "Le fichier de mise à jour Android configuré "
                            "n'est pas un APK valide"
                        ),
                    }
                ),
                502,
                headers,
            )
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
