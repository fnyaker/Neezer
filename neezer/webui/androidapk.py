# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""The Android apps' own updates, served by this server.

There are TWO Android apps now, and they must never receive each other's APK:

- the WebView app (`org.nsupysonic.app`, built by ``android.yaml``), published
  as ``neezer.apk`` and served by ``GET /api/android/apk`` — its behaviour here
  is frozen, byte for byte, because a phone that runs it may never be able to
  take a newer build;
- the native app (`fr.neezer.mobile`, built by ``mobile.yaml``), installed BESIDE
  the first one, published as ``neezer-native.apk`` (universal) and
  ``neezer-native-<abi>.apk`` (per ABI), served by
  ``GET /api/android/native-apk``.

``GET /api/version`` keeps its historical ``android`` object for the WebView app
and adds ``android.native`` beside it. An old app ignores the new key; a native
app reads ONLY the new key and answers "mise à jour indisponible" when a server
predates it — it never falls back to the old APK. Android itself would refuse
that install (different package, different key), but a server that hands out the
wrong file at all is a server that invites it.

Where a channel's file comes from, best first:

- ``[webapp] android_apk`` / ``android_native_apk``: a local file the operator
  put there (a private or fork build). Served once ``_apk_problem`` says it is
  an APK. The native path is the universal build and answers every ABI.
- the GitHub release of the declared ``android_version`` /
  ``android_native_version``, on the repository ``android_url`` points at (the
  project's by default). Fetched ONCE, by a single worker: the first request for
  that version answers ``202`` with a ``Retry-After``, the following ones serve
  the file as soon as it is on disk.

Every byte this module hands out has been checked first: a body is read whole,
its length compared with what the response announced, and the file must open as
a zip carrying Android's binary ``AndroidManifest.xml`` — on the way in (every
member's CRC read) and again on every serve (the central directory only). A
cached file that fails that check is deleted and fetched again, instead of being
served to the app for ever.

Nothing here is a trust decision: Android refuses to install an update that is
not signed with the installed app's certificate, and each app checks the package
and the signature itself before asking (so it can say why rather than failing in
the installer). The server only has to never be a way to fetch anything but the
file of the channel that was asked for.
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
NATIVE_APK_NAME = "neezer-native.apk"
# The ABIs ``mobile.yaml`` publishes splits for, in the app's own order. An
# unknown ABI is not an error: it gets the universal APK, which installs
# anywhere (the app is side-loaded).
NATIVE_ABIS = ("arm64-v8a", "armeabi-v7a", "x86_64")
APK_MIME = "application/vnd.android.package-archive"
# A release APK is ~10 MB (55 MB for the native universal one); anything past
# this is not one.
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


class _Channel:
    """One Android app's update channel: its config keys and asset names."""

    def __init__(
        self,
        *,
        version_key: str,
        local_key: str,
        asset: str,
        cache_stem: str,
    ) -> None:
        self.version_key = version_key
        self.local_key = local_key
        self.asset = asset
        self.cache_stem = cache_stem

    def asset_for(self, abi: str | None) -> str:
        if abi:
            return f"{self.cache_stem}-{abi}.apk"
        return self.asset

    def cache_name(self, version: str, abi: str | None = None) -> str:
        if abi:
            return f"{self.cache_stem}-{version}-{abi}.apk"
        return f"{self.cache_stem}-{version}.apk"

    def owns_cache_entry(self, entry: str) -> bool:
        """Whether a cached file belongs to this channel (the cleanup's rule).

        The two families share a prefix (``neezer-`` / ``neezer-native-``): the
        old channel's own match must EXCLUDE the native one, or a native fetch
        would delete the WebView app's cached update (and vice versa).
        """
        if not entry.endswith(".apk"):
            return False
        if self.cache_stem == "neezer":
            return (
                entry.startswith("neezer-")
                and not entry.startswith("neezer-native-")
            ) or entry.startswith("nsupysonic-")
        return entry.startswith("neezer-native-")


#: The WebView app's channel: its keys and asset name are the released contract,
#: and nothing in this module may change what it receives.
WEBVIEW = _Channel(
    version_key="android_version",
    local_key="android_apk",
    asset=APK_NAME,
    cache_stem="neezer",
)
#: The native app's channel, installed beside it.
NATIVE = _Channel(
    version_key="android_native_version",
    local_key="android_native_apk",
    asset=NATIVE_APK_NAME,
    cache_stem="neezer-native",
)


def declared_version(cfg: dict) -> str | None:
    return _declared_version(cfg, WEBVIEW)


def native_version(cfg: dict) -> str | None:
    return _declared_version(cfg, NATIVE)


def _declared_version(cfg: dict, channel: _Channel) -> str | None:
    version = (cfg.get(channel.version_key) or "").strip()
    return version if _VERSION_RE.match(version) else None


def _repo_of(url: str | None) -> tuple[str, str] | None:
    if not url:
        return DEFAULT_REPO
    parts = urlsplit(url)
    if parts.scheme != "https" or parts.hostname != "github.com":
        return None
    m = _REPO_RE.match(parts.path)
    return (m.group(1), m.group(2)) if m else None


def _source(cfg: dict, channel: _Channel, abi: str | None = None) -> tuple[str, str] | None:
    """``("file", path)``, ``("github", url)`` or None when nothing is offered."""
    if channel is NATIVE and abi not in NATIVE_ABIS:
        abi = None
    local = (cfg.get(channel.local_key) or "").strip()
    if local:
        # A local file is the universal build for the native channel: it answers
        # every ABI (the operator chose one file, not a release layout).
        return ("file", local) if os.path.isfile(local) else None
    version = _declared_version(cfg, channel)
    repo = _repo_of((cfg.get("android_url") or "").strip() or None)
    if not version or not repo:
        return None
    owner, name = repo
    return (
        "github",
        f"https://github.com/{owner}/{name}/releases/download/{version}/{channel.asset_for(abi)}",
    )


def apk_source(cfg: dict) -> tuple[str, str] | None:
    return _source(cfg, WEBVIEW)


def native_apk_source(cfg: dict, abi: str | None = None) -> tuple[str, str] | None:
    """The native channel's file for ``abi``, or the universal one.

    ``abi`` is only ever one of :data:`NATIVE_ABIS`; anything else (another
    architecture, a client that sends junk) gets the universal APK rather than a
    404: the universal file is what installs on a device this build never heard
    of.
    """
    return _source(cfg, NATIVE, abi if abi in NATIVE_ABIS else None)


def _cache_dir(cfg: dict) -> str:
    return os.path.join(cfg["cache_dir"], "android")


def _cache_path(cfg: dict, version: str) -> str:
    return os.path.join(_cache_dir(cfg), WEBVIEW.cache_name(version))


def _native_cache_path(cfg: dict, version: str, abi: str | None = None) -> str:
    return os.path.join(_cache_dir(cfg), NATIVE.cache_name(version, abi))


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


def _download(url: str, dest: str, channel: _Channel = WEBVIEW) -> None:
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
    # Older versions of THIS channel are never served again. The other channel's
    # files are left alone: a native update must not make the WebView app
    # download its own again (and the reverse).
    folder = os.path.dirname(dest)
    for entry in os.listdir(folder):
        path = os.path.join(folder, entry)
        if path != dest and channel.owns_cache_entry(entry):
            try:
                os.remove(path)
            except OSError:
                pass


def fetch_apk(cfg: dict) -> str:
    """Download the declared WebView release into the cache, blocking.

    The worker's whole job (the route never calls it): a slow GitHub used to
    hold the request thread — and the global lock behind it, so every other
    device waited too — for the length of a 10 MB download.
    """
    source = apk_source(cfg)
    if source is None or source[0] != "github":
        raise OSError("no release to fetch")
    return _fetch_asset(source[1], _cache_path(cfg, declared_version(cfg)), WEBVIEW)


def fetch_native_apk(cfg: dict, abi: str | None = None) -> str:
    """Download the declared native release for ``abi`` (universal when absent)."""
    source = native_apk_source(cfg, abi)
    if source is None or source[0] != "github":
        raise OSError("no release to fetch")
    wanted = abi if abi in NATIVE_ABIS else None
    dest = _native_cache_path(cfg, native_version(cfg), wanted)
    fallback = None
    if wanted is not None:
        universal = native_apk_source(cfg, None)
        if universal is not None and universal[0] == "github":
            fallback = (universal[1], _native_cache_path(cfg, native_version(cfg), None))
    return _fetch_with_fallback(source[1], dest, NATIVE, fallback)


def _fetch_asset(where: str, dest: str, channel: _Channel = WEBVIEW) -> str:
    os.makedirs(os.path.dirname(dest), exist_ok=True)
    if os.path.isfile(dest):
        problem = _apk_problem(dest)
        if problem is None:
            return dest
        _discard(dest, problem)
    _download(where, dest, channel)
    return dest


#: How long a failed fetch is remembered before a tap may start a new one.
APK_RETRY = 30.0
#: What a client is told to wait while a fetch runs: an APK is ~10 MB, and the
#: app asks again rather than holding its own thread on a socket.
APK_WAIT = 5

_apk_lock = threading.Lock()
_apk_jobs = {}  # dest -> {"state": "fetching"|"ready"|"failed", "error", "at", "thread"}


def _serve_file(cfg: dict, channel: _Channel, abi: str | None = None) -> str | None:
    source = _source(cfg, channel, abi)
    if source is None:
        return None
    kind, where = source
    if kind == "file":
        problem = _apk_problem(where)
        if problem is None:
            return where
        logger.warning("Fichier de mise à jour Android inutilisable (%s) : %s", problem, where)
        return None
    version = _declared_version(cfg, channel)
    wanted = abi if channel is NATIVE and abi in NATIVE_ABIS else None
    dest = (
        _native_cache_path(cfg, version, wanted)
        if channel is NATIVE
        else _cache_path(cfg, version)
    )
    if not os.path.isfile(dest):
        # A release cut before the per-ABI splits carried only the universal
        # APK, and the worker cached it there: it answers this ABI too.
        if channel is NATIVE and wanted is not None:
            universal = _native_cache_path(cfg, version, None)
            if os.path.isfile(universal) and _apk_problem(universal) is None:
                return universal
        return None
    problem = _apk_problem(dest)
    if problem is None:
        return dest
    _discard(dest, problem)
    return None


def cached_apk(cfg: dict) -> str | None:
    """The file to serve right now, without asking anyone."""
    return _serve_file(cfg, WEBVIEW)


def cached_native_apk(cfg: dict, abi: str | None = None) -> str | None:
    return _serve_file(cfg, NATIVE, abi)


def _job(cfg: dict, channel: _Channel, abi: str | None = None) -> dict | None:
    version = _declared_version(cfg, channel)
    if version is None or not cfg.get("cache_dir"):
        return None
    dest = (
        _native_cache_path(cfg, version, abi if abi in NATIVE_ABIS else None)
        if channel is NATIVE
        else _cache_path(cfg, version)
    )
    with _apk_lock:
        job = _apk_jobs.get(dest)
        return dict(job) if job is not None else None


def apk_job(cfg: dict) -> dict | None:
    """The last fetch of the declared WebView version: state, cause, thread."""
    return _job(cfg, WEBVIEW)


def native_apk_job(cfg: dict, abi: str | None = None) -> dict | None:
    return _job(cfg, NATIVE, abi)


def _ensure(cfg: dict, channel: _Channel, abi: str | None = None) -> str | None:
    """The path to serve, or None after making sure ONE worker fetches it."""
    path = _serve_file(cfg, channel, abi)
    if path is not None:
        return path
    source = _source(cfg, channel, abi)
    if source is None or source[0] != "github":
        return None
    version = _declared_version(cfg, channel)
    wanted = abi if channel is NATIVE and abi in NATIVE_ABIS else None
    dest = (
        _native_cache_path(cfg, version, wanted)
        if channel is NATIVE
        else _cache_path(cfg, version)
    )
    where = source[1]
    # A release cut before the per-ABI splits carried only the universal APK:
    # the worker falls back to it once, and to nothing else.
    fallback = None
    if channel is NATIVE and wanted is not None:
        universal = _source(cfg, NATIVE, None)
        if universal is not None and universal[0] == "github":
            fallback = (
                universal[1],
                _native_cache_path(cfg, version, None),
            )
    with _apk_lock:
        job = _apk_jobs.get(dest)
        if job is not None:
            if job["state"] == "fetching":
                return None
            if time.monotonic() - job["at"] < APK_RETRY:
                return None
        job = {"state": "fetching", "error": None, "at": time.monotonic(), "thread": None}
        job["thread"] = threading.Thread(
            target=_fetch,
            args=(where, dest, channel, fallback),
            name=channel.cache_stem,
            daemon=True,
        )
        _apk_jobs[dest] = job
    job["thread"].start()
    return None


def ensure_apk(cfg: dict) -> str | None:
    return _ensure(cfg, WEBVIEW)


def ensure_native_apk(cfg: dict, abi: str | None = None) -> str | None:
    return _ensure(cfg, NATIVE, abi)


def _fetch_with_fallback(
    where: str,
    dest: str,
    channel: _Channel,
    fallback: tuple[str, str] | None,
) -> str:
    """``_fetch_asset``, with ONE universal fallback on a 404 for a split.

    A release cut before the per-ABI splits existed carries only
    ``neezer-native.apk``; a split that is simply absent from the asset list is a
    404 from GitHub, and the universal file installs on that device too.
    """
    try:
        return _fetch_asset(where, dest, channel)
    except requests.HTTPError as error:
        if fallback is None or error.response is None or error.response.status_code != 404:
            raise
        logger.info("Pas de split dans cette release : APK natif universel")
        return _fetch_asset(fallback[0], fallback[1], channel)


def _fetch(
    where: str,
    dest: str,
    channel: _Channel = WEBVIEW,
    fallback: tuple[str, str] | None = None,
) -> None:
    from ..deezer import workload

    workload.renice(10)
    error = None
    try:
        _fetch_with_fallback(where, dest, channel, fallback)
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
            done = os.path.isfile(dest) or (
                fallback is not None and os.path.isfile(fallback[1])
            )
            job["state"] = "ready" if done else "failed"
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


def _native_abi(requested: str | None) -> str | None:
    return requested if requested in NATIVE_ABIS else None


@webapi.route("/android/native-apk")
@login_required
def android_native_apk():
    """The NATIVE app's channel — never the WebView APK, even when it is newer.

    ``?abi=`` is the device's own ABI: the server serves that split when the
    release carries it (a third of the bytes of the universal APK, rule 3) and
    the universal one otherwise.
    """
    from flask import request

    cfg = current_app.config["WEBAPP"]
    abi = _native_abi(request.args.get("abi"))
    version = native_version(cfg)
    source = native_apk_source(cfg, abi)
    path = ensure_native_apk(cfg, abi)
    if path is None:
        if source is None:
            return (
                jsonify(
                    {
                        "error": (
                            "Ce serveur ne publie pas l'application Android native "
                            "(mise à jour indisponible)"
                        )
                    }
                ),
                404,
            )
        headers = {"Retry-After": str(APK_RETRY)}
        if version:
            headers["X-NS-Version"] = version
        if source[0] == "file":
            return (
                jsonify(
                    {
                        "state": "failed",
                        "error": (
                            "Le fichier de mise à jour Android natif configuré "
                            "n'est pas un APK valide"
                        ),
                    }
                ),
                502,
                headers,
            )
        job = native_apk_job(cfg, abi) or {}
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
        path, mimetype=APK_MIME, as_attachment=True, download_name=NATIVE_APK_NAME,
        conditional=True,
    )
    if version:
        response.headers["X-NS-Version"] = version
    return response
