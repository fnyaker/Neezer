#!/usr/bin/env python3
"""D17's journey bench: every screen, on the worst realistic library and link.

What it measures, and how (the numbers land in the PR and in
`docs/agents/desktop.md`):

1. **Requests and response bytes per screen** — the app runs against
   `tools/perf_api.py --serve` (8 000 tracks, 4 000 favourites, a 4 000-track
   playlist, real sleeves) either directly or through `throttle.py` (300 ms RTT,
   1 Mb/s, 2 % loss). Each screen is opened with `--route <token>` so the run is
   the same one every time; the proxy's own table says what that screen asked
   for and how many bytes came back.
2. **Click to sound** for the three states a track can be in: in the device's
   play cache, archived on the server (its Opus transcode already cached), and
   cold (the server generates the stream as it sends it). The bench clicks the
   row and reads `--bench-events`: `load` is the gesture, `started` is the
   engine's first sample, both on the app's own clock.
3. **Frame time** during navigation and scrolling: `--frame-log` records every
   painted frame; `frame_stats.py` turns it into p50/p95/max. Under Xvfb the
   PRESENT rate is the software renderer's (24–30 fps), so that number is a
   floor, not the panel's; the app's own frame-build time is the part this
   machine can compare across changes.
4. **RSS** at rest, while playing and with the visualiser, at each window size.
5. **Zero underruns over an hour** with downloads and a transcription running:
   `soak.py` drives that, `--bench-status` records the engine's counter.
6. **The upgrade path**: an old store opens (`nz-cache`'s own tests), and a
   server without a route makes the app hide the feature (`--older-server`,
   which 404s one feature's routes through the proxy).

The app is a RELEASE build (`desktop/target/release/neezer-desktop`), on an
Xvfb this script started (`:130` and above), with its own XDG directories under
the scratch directory: the person's Hyprland session, PipeWire volume and
browser are never touched.

Dependencies: Python 3.10+, `python-xlib` for the X11 driving
(`pip install python-xlib`), ImageMagick `convert`, `ffmpeg` for the FLAC
fixtures, and the project's own venv for the server (`NZ_PYTHON`).
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[2]
DESKTOP = REPO / "desktop"

# The throwaway account `tools/perf_api.py` seeds for the bench, and nothing
# else: the server it logs into lives for one run, on loopback, with a library
# the bench made. Kept in one place so no call site invents another.
BENCH_PASSWORD = "Bench1"

# Every URL this bench asks is the perf_api server or its proxy, both bound to
# loopback: `local_url` refuses the rest.
LOCAL_HOSTS = frozenset({"127.0.0.1", "localhost", "::1"})

# The screens the app has, and the token `Route::spec` gives each one. Settings
# and the detail pages need ids, which `--route` cannot know: the bench fills
# them from the running server (a favourite's track, the first playlist, the
# first show) before the screen loop.
STATIC_SCREENS = [
    "home",
    "search",
    "library:playlists",
    "library:favorites",
    "library:artists",
    "library:albums",
    "library:folders",
    "library:uploads",
    "library:dislikes",
    "library:unavailable",
    "podcasts",
    "releases",
    "recap",
    "history",
    "genres",
    "import",
    "party",
    "remote",
    "now-playing",
    "queue",
    "viz",
    "settings:lecture-et-son",
    "settings:apparence",
    "settings:visualiseur",
    "settings:telechargements",
    "settings:decouverte",
    "settings:sources",
    "settings:compte",
    "settings:transcription-locale",
    "settings:assistance",
    "settings:raccourcis",
    "settings:utilisateurs",
    "settings:serveur",
]


def log(*parts) -> None:
    print("·", *parts, flush=True)


def free_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def local_url(url: str) -> str:
    """`url` if it names this machine's own HTTP server, else a refusal.

    urllib also speaks `file://`, and a URL that came back in a response (or
    from an argument) must never turn the bench into a file reader or a request
    to another host. Everything the bench asks is `perf_api.py` or its proxy.
    """
    parts = urllib.parse.urlsplit(url)
    if parts.scheme != "http" or parts.hostname not in LOCAL_HOSTS:
        raise ValueError(f"refusing a non-local URL: {url}")
    return url


def local_open(url: str, timeout: float, **request) -> object:
    """The response for `url`, which `local_url` has checked, as a `Request`.

    `**request` is what `urllib.request.Request` takes: `data`, `headers`,
    `method`. One place builds the request, so one place to audit: the two calls
    sit on one line because they are one request.
    """
    return urllib.request.urlopen(urllib.request.Request(local_url(url), **request), timeout=timeout)  # nosemgrep: python.lang.security.audit.dynamic-urllib-use-detected.dynamic-urllib-use-detected -- local_url() refuses every URL that is not http on this machine


def wait_http(url: str, timeout: float = 60.0) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            with local_open(url, timeout=2):
                return True
        except (urllib.error.URLError, ConnectionError, TimeoutError):
            time.sleep(0.1)
    return False


class Server:
    """`tools/perf_api.py --serve`, the same bench every other lot measured on."""

    def __init__(self, python: str, covers: Path | None, releases: int = 0) -> None:
        self.python = python
        self.covers = covers
        self.releases = releases
        self.process: subprocess.Popen | None = None
        self.port = 0
        self.ids: dict = {}
        # The synthetic library is a throwaway directory `perf_api.py` makes
        # and removes; the bench finds it through the process's own open files
        # so a second worker's bench on this shared machine is never touched.
        self.workdir: Path | None = None

    def start(self) -> None:
        command = [
            self.python,
            str(REPO / "tools" / "perf_api.py"),
            "--serve",
            "--archived",
            "--podcasts",
            "--seed-lyrics",
            "--seed-genre-scores",
            "--recap",
            "--admin-users",
            "3",
            "--admin-invites",
            "2",
            "--onboarding",
        ]
        if self.covers:
            command += ["--covers", str(self.covers)]
        if self.releases:
            # `perf_api.py --releases` seeds the feed and lifts its cap in the
            # bench's own process: the desktop's Nouveautés page is measured on
            # thousands of releases, not on the shipped server's 120.
            command += ["--releases", str(self.releases)]
        self.process = subprocess.Popen(
            command,
            cwd=REPO,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            bufsize=1,
        )
        deadline = time.monotonic() + 180
        while time.monotonic() < deadline:
            line = self.process.stdout.readline()
            if not line:
                break
            if line.startswith("READY "):
                _, port, ids = line.split(" ", 2)
                self.port = int(port)
                self.ids = json.loads(ids)
                self.workdir = self._workdir()
                return
        raise RuntimeError("perf_api.py never said READY")

    def _workdir(self) -> Path | None:
        """The library `perf_api.py` made, from its open SQLite file."""
        base = Path(f"/proc/{self.process.pid}/fd")
        try:
            for entry in base.iterdir():
                try:
                    target = os.readlink(entry)
                except OSError:
                    continue
                if "/nsperf-" in target and target.endswith("db.sqlite"):
                    return Path(target).parent
        except OSError:
            pass
        return None

    def url(self) -> str:
        return f"http://127.0.0.1:{self.port}"

    def stop(self) -> None:
        if self.process and self.process.poll() is None:
            self.process.send_signal(signal.SIGTERM)
            try:
                self.process.wait(timeout=20)
            except subprocess.TimeoutExpired:
                self.process.kill()


class Proxy:
    """`throttle.py` in front of the server: the degraded link, no root."""

    def __init__(self, upstream: str, scratch: Path) -> None:
        self.upstream = upstream
        self.scratch = scratch
        self.process: subprocess.Popen | None = None
        self.port = 0
        self.stats_port = 0
        self.deny = ""

    def start(self, latency_ms: float, rate_bps: int, loss: float) -> None:
        self.port = free_port()
        self.stats_path = self.scratch / "proxy-stats.jsonl"
        self.stats_port = free_port()
        self.process = subprocess.Popen(
            [
                sys.executable,
                str(HERE / "throttle.py"),
                "--listen",
                f"127.0.0.1:{self.port}",
                "--upstream",
                self.upstream,
                "--stats",
                f"127.0.0.1:{self.stats_port}",
                "--latency-ms",
                str(latency_ms),
                "--rate-bps",
                str(rate_bps),
                "--loss",
                str(loss),
            ]
            + (["--deny", self.deny] if self.deny else []),
            stdout=subprocess.PIPE,
            text=True,
        )
        line = self.process.stdout.readline()
        if not line.startswith("READY"):
            raise RuntimeError(f"throttle.py did not start: {line!r}")
        self.port = int(line.split()[1])

    def url(self) -> str:
        return f"http://127.0.0.1:{self.port}"

    def stats(self, reset: bool = False) -> dict:
        path = "/reset" if reset else "/stats"
        with local_open(f"http://127.0.0.1:{self.stats_port}{path}", timeout=30) as answer:
            return json.load(answer)

    def snapshots(self) -> dict:
        return self.stats()

    def stop(self) -> None:
        if self.process and self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.process.kill()
        marker = getattr(self, "marker", None)
        if marker is not None:
            marker.unlink(missing_ok=True)


class Xvfb:
    """Our own X server: never the person's Hyprland session."""

    def __init__(self, scratch: Path) -> None:
        self.scratch = scratch
        self.process: subprocess.Popen | None = None
        self.display = ""

    def start(self, width: int = 2600, height: int = 1700, base: int = 130) -> str:
        # `X{number}-lock` alone is not enough: two benches starting together
        # both see it absent, both spawn `Xvfb :130`, one wins and the other
        # waits on a lock that is not its own (measured: a soak and a bench
        # killed each other's display). An O_EXCL marker per number serialises
        # the choice.
        for number in range(base, base + 60):
            marker = Path(f"/tmp/nz-d17-display-{number}")
            try:
                os.close(os.open(marker, os.O_CREAT | os.O_EXCL | os.O_WRONLY))
            except FileExistsError:
                continue
            self.marker = marker
            if Path(f"/tmp/.X{number}-lock").exists():
                marker.unlink(missing_ok=True)
                continue
            process = subprocess.Popen(
                [
                    "Xvfb",
                    f":{number}",
                    "-screen",
                    "0",
                    f"{width}x{height}x24",
                    "-dpi",
                    "96",
                    "-nolisten",
                    "tcp",
                ],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            for _ in range(100):
                if Path(f"/tmp/.X{number}-lock").exists():
                    self.process = process
                    self.display = f":{number}"
                    return self.display
                if process.poll() is not None:
                    break
                time.sleep(0.05)
            if process.poll() is None:
                process.terminate()
            marker.unlink(missing_ok=True)
        raise RuntimeError(f"no free X display in :{base}..:{base + 59}")

    def stop(self) -> None:
        if self.process and self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.process.kill()
        marker = getattr(self, "marker", None)
        if marker is not None:
            marker.unlink(missing_ok=True)


class App:
    """One `neezer-desktop` process with its own XDG tree and measurement logs."""

    def __init__(self, binary: Path, server_url: str, scratch: Path,
                 display: str, bus_address: str | None) -> None:
        self.binary = binary
        self.server_url = server_url
        self.scratch = scratch
        self.display = display
        self.bus_address = bus_address
        self.process: subprocess.Popen | None = None
        self.lines: list[str] = []
        self._reader: object | None = None
        self._lock: object | None = None
        self.events = scratch / "events.jsonl"
        self.frames = scratch / "frames.log"
        self.status = scratch / "status.jsonl"

    def env(self) -> dict:
        env = dict(os.environ)
        env.update(
            {
                "DISPLAY": self.display,
                "NEEZER_SERVER": self.server_url,
                # Never the person's real XDG tree: the store, the caches and
                # the session jar of a measurement are the bench's.
                "XDG_DATA_HOME": str(self.scratch / "xdg" / "data"),
                "XDG_CACHE_HOME": str(self.scratch / "xdg" / "cache"),
                "XDG_STATE_HOME": str(self.scratch / "xdg" / "state"),
                "XDG_CONFIG_HOME": str(self.scratch / "xdg" / "config"),
                "GPUI_X11_SCALE_FACTOR": "1",
                "RUST_BACKTRACE": "1",
            }
        )
        if self.bus_address:
            env["DBUS_SESSION_BUS_ADDRESS"] = self.bus_address
        # D17's hard rule: the app must never reach the person's session. GPUI
        # prefers Wayland when `WAYLAND_DISPLAY` is set, whatever `DISPLAY`
        # says — measured: the window opened on the developer's Hyprland
        # session while the bench waited for it on Xvfb. Both variables go.
        for variable in ("WAYLAND_DISPLAY", "WAYLAND_SOCKET", "HYPRLAND_INSTANCE_SIGNATURE"):
            env.pop(variable, None)
        env["XDG_SESSION_TYPE"] = "x11"
        return env

    def start(self, route: str, size: tuple[int, int], extra: list[str] | None = None) -> None:
        for path in (self.events,):
            if path.exists():
                path.unlink()
        command = [
            str(self.binary),
            "--route",
            route,
            "--window-size",
            f"{size[0]}x{size[1]}",
            "--bench-events",
            str(self.events),
            "--frame-log",
            str(self.frames),
            "--bench-status",
            str(self.status),
        ]
        if extra:
            command += extra
        self.process = subprocess.Popen(
            command,
            env=self.env(),
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            bufsize=1,
        )
        # A reader THREAD, not a blocking `readline` on the caller: an app that
        # never prints (a crash on the login screen) must time out, not hang
        # the bench.
        self.lines = []
        self._lock = __import__("threading").Lock()
        self._reader = __import__("threading").Thread(
            target=self._drain, daemon=True
        )
        self._reader.start()

    def _drain(self) -> None:
        assert self.process is not None and self.process.stdout is not None
        for line in self.process.stdout:
            with self._lock:
                self.lines.append(line.rstrip("\n"))

    def read_until(self, predicate, timeout: float = 60.0) -> str | None:
        """Wait for a line of the app's own output; every line is kept."""
        deadline = time.monotonic() + timeout
        seen = 0
        while time.monotonic() < deadline:
            with self._lock:
                lines = self.lines[seen:]
                seen = len(self.lines)
            for line in lines:
                if predicate(line):
                    return line
            time.sleep(0.02)
        return None

    def rss_kb(self) -> int:
        if self.process is None:
            return 0
        try:
            text = Path(f"/proc/{self.process.pid}/status").read_text()
        except OSError:
            return 0
        for line in text.splitlines():
            if line.startswith("VmRSS:"):
                return int(line.split()[1])
        return 0

    def events_log(self) -> list[dict]:
        if not self.events.exists():
            return []
        rows = []
        for line in self.events.read_text().splitlines():
            try:
                rows.append(json.loads(line))
            except json.JSONDecodeError:
                pass
        return rows

    def stop(self) -> None:
        if self.process and self.process.poll() is None:
            self.process.send_signal(signal.SIGTERM)
            try:
                self.process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                self.process.kill()
        self.process = None


def seed_session(server_url: str, scratch: Path, user: str = "bench",
                 *, password: str) -> None:
    """Log in with urllib and write the jar the app reads at start.

    The app's jar is `{"<origin>": [[name, value], …]}` (nz-api's
    `session.json`), keyed by ORIGIN — so the cookie is written under the
    proxy's origin when the app talks through the proxy, and the Flask session
    cookie is the same value either way.

    `password` is the bench's own throwaway account (`BENCH_PASSWORD`), never a
    real one: the server exists for this run and is bound to loopback. It has no
    default on purpose — a default password is one nobody reads, and this bench
    has exactly one.
    """
    import http.cookiejar

    local_url(server_url)  # the cookie jar is only ever written to our own server
    # A login on a busy machine (two benches, downloads, whisper) can take
    # longer than one 30 s socket timeout; three tries, never a silent skip.
    last_error: Exception | None = None
    for attempt in range(3):
        jar = http.cookiejar.CookieJar()
        opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar))
        body = json.dumps({"username": user, "password": password}).encode()
        request = urllib.request.Request(
            f"{server_url}/api/login",
            data=body,
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        try:
            with opener.open(request, timeout=60) as answer:
                if answer.status == 200:
                    break
                last_error = RuntimeError(f"login failed: {answer.status}")
        except (TimeoutError, urllib.error.URLError) as error:
            last_error = error
            time.sleep(1.0 + attempt)
    else:
        raise RuntimeError(f"login never answered: {last_error}")
    cookies = [[cookie.name, cookie.value] for cookie in jar]
    if not cookies:
        raise RuntimeError("login set no cookie")
    state = scratch / "xdg" / "state" / "neezer"
    state.mkdir(parents=True, exist_ok=True)
    path = state / "session.json"
    path.write_text(json.dumps({server_url.rstrip("/"): cookies}))
    path.chmod(0o600)


def api(url: str, path: str, cookie: str | None = None) -> dict:
    headers = {"Cookie": cookie} if cookie else {}
    with local_open(url + path, timeout=60, headers=headers) as answer:
        return json.load(answer)


def login_cookie(url: str) -> str:
    body = json.dumps({"username": "bench", "password": BENCH_PASSWORD}).encode()
    with local_open(
        f"{url}/api/login",
        timeout=30,
        data=body,
        headers={"Content-Type": "application/json"},
        method="POST",
    ) as answer:
        header = answer.headers.get("Set-Cookie", "")
    return header.split(";", 1)[0]


def make_flacs(scratch: Path, count: int = 3, seconds: int = 90) -> list[Path]:
    """Real FLACs: the app decodes FLAC and plays Opus transcodes, never WAV.

    They are written into the bench's ARCHIVE (`archive/journey/`), because the
    server serves only files under its archive: a row pointed at a file outside
    it answers 503 "not archived" and nothing plays (measured).
    """
    music = scratch / "music"
    music.mkdir(parents=True, exist_ok=True)
    paths = []
    for index in range(count):
        path = music / f"tone-{index}.flac"
        if not path.exists():
            subprocess.run(
                [
                    "ffmpeg",
                    "-v",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    f"sine=frequency={330 + 110 * index}:duration={seconds}:sample_rate=44100",
                    "-c:a",
                    "flac",
                    str(path),
                ],
                check=True,
            )
        paths.append(path)
    return paths


class Bus:
    """A private session bus, so MPRIS never shadows the person's player."""

    def __init__(self) -> None:
        self.process: subprocess.Popen | None = None
        self.address = ""

    def start(self) -> str:
        result = subprocess.run(
            ["dbus-daemon", "--session", "--print-address=1", "--print-pid=1", "--fork"],
            capture_output=True,
            text=True,
            check=True,
        )
        lines = result.stdout.splitlines()
        self.address = lines[0].strip()
        self.pid = int(lines[1]) if len(lines) > 1 else 0
        return self.address

    def stop(self) -> None:
        if getattr(self, "pid", 0):
            try:
                os.kill(self.pid, signal.SIGTERM)
            except OSError:
                pass


def point_track_at(workdir: Path, track_id: str, flac: Path) -> None:
    """Make one bench row play a REAL FLAC.

    `perf_api.py` archives one-second silent WAVs (its own suites only need the
    row to have a file), and the desktop app decodes FLAC/Opus/MP3/MP4 — never
    WAV. The row's `path` is the one thing `/api/stream` reads from the archive,
    so the bench points it at a real file through the same SQLite store the
    server has open. The `last_modification` stamp is what `/genre/candidates`
    and the local search read; both are already set by `--archived`.
    """
    import sqlite3

    # The store keeps a row id as 32 hex characters, WITHOUT the dashes the API
    # prints (`89e80170-adc3-…` → `89e80170adc3…`). Measured: an UPDATE keyed by
    # the dashed form matched nothing, so the row kept its silent WAV and every
    # stream answered 503.
    key = track_id.replace("-", "")
    database = workdir / "db.sqlite"
    with sqlite3.connect(database, timeout=30) as connection:
        changed = connection.execute(
            "UPDATE track SET path = ? WHERE id = ?", (str(flac), key)
        ).rowcount
        connection.commit()
    if not changed:
        raise RuntimeError(f"no track row {track_id} in {database}")


def screen_set(server_url: str) -> list[str]:
    """The static screens plus the detail pages the server's own data names."""
    cookie = login_cookie(server_url)
    screens = list(STATIC_SCREENS)
    try:
        favourites = api(server_url, "/api/me/favorites", cookie)
        rows = favourites.get("tracks") or favourites.get("results") or []
        if rows:
            track = rows[0]
            screens.append(f"artist:{track['artist']['deezer_id']}")
            if track.get("album"):
                screens.append(f"album:{track['album']['deezer_id']}")
            screens.append(f"credits:track:{track['deezer_id']}")
            if track["artist"].get("deezer_id"):
                screens.append(f"credits:person:{track['artist']['deezer_id']}")
            screens.append(
                "playlist:" + (api(server_url, "/api/me/playlists", cookie)["playlists"][0]["id"])
            )
    except (KeyError, IndexError, TypeError, urllib.error.HTTPError) as error:
        log("detail pages skipped:", error)
    try:
        shows = api(server_url, "/api/podcasts", cookie)["channels"]
        if shows:
            screens.append(f"show:{shows[0]['id']}")
    except (KeyError, IndexError, TypeError, urllib.error.HTTPError):
        pass
    return screens


def settle(proxy: "Proxy", quiet_ms: float = 500.0, timeout: float = 25.0) -> dict:
    """Wait until the link has been quiet for `quiet_ms`, then read the window.

    A screen's requests arrive in bursts (a page, then its sleeves); the table
    must be that screen's work, so the window closes when nothing new has been
    asked for — not after a fixed sleep, which would either cut a slow link's
    answer or count the next screen's.
    """
    started = time.monotonic()
    last_seen = 0
    quiet_since = None
    while time.monotonic() - started < timeout:
        snapshot = proxy.stats()
        count = snapshot["requests"]
        if count != last_seen:
            last_seen = count
            quiet_since = time.monotonic()
        elif quiet_since and (time.monotonic() - quiet_since) * 1000 >= quiet_ms:
            return snapshot
        time.sleep(0.05)
    return proxy.stats()


def measure_screen(app: "App", screen: "Screen", proxy: "Proxy", route: str,
                   size: tuple[int, int], shot: Path | None,
                   drive: str = "", panel: bool = False,
                   quiet_ms: float = 700.0) -> dict:
    """One screen: its request table, its frame log, its picture.

    The proxy window opens BEFORE the process starts: the first visit of a
    screen is what a slow link pays (an app store that already holds the
    library answers 0 requests — that is the second visit, measured apart).
    """
    proxy.stats(reset=True)
    app.start(route, size)
    if app.read_until(lambda line: line.startswith("bench logged_in"), timeout=120) is None:
        tail = "\n".join(app.lines[-12:])
        raise RuntimeError(f"{route}: the session was never restored\n{tail}")
    window = screen.wait_window()
    screen.focus(window)
    snapshot = settle(proxy, quiet_ms=quiet_ms, timeout=90.0)
    if panel:
        # The 420 px side panel (`q`): the queue beside the screen, which is
        # where a 1280×800 window gets cramped. Measured as its own row.
        screen.keys("q")
        time.sleep(0.6)
        snapshot = settle(proxy, quiet_ms=500.0, timeout=20.0)
    time.sleep(0.4)
    if shot is not None:
        screen.screenshot(str(shot), window)
    rss = app.rss_kb()
    driven = {}
    if drive:
        driven = drive_phase(app, screen, window, drive)
    geometry = window.get_geometry()
    rows = app.events_log()
    app.stop()
    frames = frame_stats(app.frames)
    return {
        "route": route,
        "size": f"{size[0]}x{size[1]}",
        "window": f"{geometry.width}x{geometry.height}",
        "requests": snapshot["requests"],
        "bytes": snapshot["bytes"],
        "paths": snapshot.get("paths", {}),
        "rss_kb": rss,
        "frames": frames,
        "driven": driven,
        "events": rows,
    }


def drive_phase(app: "App", screen: "Screen", window, mode: str) -> dict:
    """Paint under load: 40 `Page suivante`s, or 6 s of the visualiser.

    Under Xvfb the present rate is the software renderer's, so what is reported
    is the app's own frame-build time and the interval the X server delivered;
    the panel's number comes from `frame_bench.sh`.
    """
    if app.frames.exists():
        # Truncate, never unlink: the app opened the file once and appends to
        # that handle, so a deleted path would leave a new file unwritten.
        app.frames.write_text("")
    started = time.monotonic()
    if mode == "scroll":
        for _ in range(40):
            screen.keys("pagedown")
            time.sleep(0.05)
    elif mode == "wheel":
        # 40 wheel notches over the middle of the page, 50 ms apart (the same
        # cadence as the `Page suivante` drive below): the input keeps the app
        # painting, so the interval between frames is the rate it SUSTAINS
        # under a scroll — a build that cannot keep up paints nothing at all.
        # the same input on every build, so the frames are comparable across a
        # change that moves the rows into a list. The spacing is what lets a
        # build that spends seconds per frame still paint a few — a page frozen
        # for the whole window would report zero frames instead of a number.
        geometry = window.get_geometry()
        at = (geometry.x + geometry.width // 2, geometry.y + geometry.height // 3)
        for _ in range(40):
            screen.wheel(1, down=True, at=at)
            time.sleep(0.05)
    elif mode == "viz":
        screen.keys("v")
        _wait_frames(app, 400, timeout=8.0)
        time.sleep(4.0)
    elif mode == "select":
        for _ in range(30):
            screen.keys("down")
            time.sleep(0.03)
    return {
        "mode": mode,
        "seconds": round(time.monotonic() - started, 2),
        "frames": frame_stats(app.frames),
    }


def _wait_frames(app: "App", count: int, timeout: float) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if app.frames.exists() and app.frames.read_text().count("frame ") >= count:
            return
        time.sleep(0.02)


def frame_stats(path: Path) -> dict:
    """p50/p95/max of the frame interval and of the build time, in ms."""
    if not path.exists():
        return {}
    intervals: list[float] = []
    builds: list[float] = []
    previous = None
    for line in path.read_text().splitlines():
        parts = line.split()
        if len(parts) != 3 or parts[0] != "frame":
            continue
        moment = int(parts[1])
        builds.append(int(parts[2]) / 1000.0)
        if previous is not None:
            intervals.append(moment - previous)
        previous = moment
    if not intervals:
        return {"frames": len(builds)}
    # A frame that arrives after a quarter of a second was not painted by a
    # continuous animation; keeping those in the interval sample would report an
    # fps of the app's idle time, which is not what the number means.
    continuous = [value for value in intervals if value <= 250]
    return {
        "frames": len(builds),
        "interval_p50_ms": percentile(continuous or intervals, 50),
        "interval_p95_ms": percentile(continuous or intervals, 95),
        "interval_max_ms": max(continuous or intervals),
        "fps_continuous": round(1000 / (sum(continuous) / len(continuous)), 1)
        if continuous
        else 0,
        "build_p50_ms": percentile(builds, 50),
        "build_p95_ms": percentile(builds, 95),
        "build_max_ms": max(builds),
    }


def percentile(values: list[float], fraction: float) -> float:
    if not values:
        return 0.0
    ordered = sorted(values)
    index = min(len(ordered) - 1, max(0, round(fraction / 100 * (len(ordered) - 1))))
    return round(ordered[index], 2)


def start_bus() -> Bus:
    bus = Bus()
    bus.start()
    return bus


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--python", default=os.environ.get("NZ_PYTHON", "python3"),
                        help="the Python that can import neezer (the project venv)")
    parser.add_argument("--binary", default=str(DESKTOP / "target/release/neezer-desktop"))
    parser.add_argument("--scratch", default="")
    parser.add_argument("--sizes", default="2560x1600,1280x800")
    parser.add_argument("--screens", default="all",
                        help="comma-separated route tokens, or 'all'")
    parser.add_argument("--shots", default="", help="directory for the captures")
    parser.add_argument("--degraded", action="store_true",
                        help="talk through throttle.py (300 ms, 1 Mb/s, 2 %% loss)")
    parser.add_argument("--out", default="", help="where the JSON result goes")
    parser.add_argument("--skip-screens", action="store_true")
    parser.add_argument("--display-base", type=int, default=160,
                        help="first X display this bench may take (:160 and up)")
    parser.add_argument("--settle-ms", type=float, default=700.0,
                        help="how long the link must be quiet to call a screen loaded; "
                             "a degraded link needs more than the lane gaps (250 ms "
                             "Prefetch, 1 s Bulk) plus its own RTT — 2500 there")
    parser.add_argument("--panel", action="store_true",
                        help="open the 420 px queue panel before each capture")
    parser.add_argument("--deny", default="",
                        help="answer 404 for every request whose path starts with "
                             "this prefix: an older server that lacks a feature")
    parser.add_argument("--releases", type=int, default=0, metavar="N",
                        help="seed N releases (and lift perf_api's own caps) so the "
                             "releases screen is measured on a few thousand of them")
    args = parser.parse_args()

    scratch = Path(args.scratch) if args.scratch else Path(tempfile.mkdtemp(prefix="nz-d17-"))
    scratch.mkdir(parents=True, exist_ok=True)
    shots = Path(args.shots) if args.shots else None
    if shots:
        shots.mkdir(parents=True, exist_ok=True)
    script_path = HERE / "drive.py"
    sys.path.insert(0, str(HERE))
    import drive  # noqa: E402  (needs python-xlib, documented above)

    covers = make_cover(scratch)
    server = Server(args.python, covers, releases=args.releases)
    server.start()
    log("server", server.url(), "workdir", server.workdir)
    # ALWAYS through the proxy: on a local link it is a counting tap (0 ms,
    # no cap), and `--degraded` turns it into the 300 ms / 1 Mb/s / 2 % link.
    # One code path, so the two tables are the same measurement.
    proxy = Proxy(f"127.0.0.1:{server.port}", scratch)
    proxy.deny = args.deny
    if args.degraded:
        proxy.start(latency_ms=150, rate_bps=1_000_000, loss=0.02)
    else:
        proxy.start(latency_ms=0.0, rate_bps=0, loss=0.0)
    app_url = proxy.url()
    log("link", app_url, "->", server.url(), "degraded" if args.degraded else "local")
    xvfb = Xvfb(scratch)
    display = xvfb.start(base=args.display_base)
    log("display", display)
    bus = start_bus()
    results: dict = {"server": server.url(), "app": app_url, "display": display,
                     "degraded": bool(proxy), "screens": []}
    reporter = lambda kind, payload: None  # noqa: E731
    screen = None
    try:
        seed_session(app_url, scratch, password=BENCH_PASSWORD)
        screens = screen_set(server.url())
        if args.screens != "all":
            screens = [token.strip() for token in args.screens.split(",") if token.strip()]
        screens = list(dict.fromkeys(screens))
        log(len(screens), "screens")
        for index, route in enumerate(screens):
            for size in parse_sizes(args.sizes):
                if args.skip_screens:
                    continue
                token = route.replace(":", "-").replace("/", "-")
                panel = "-panneau" if args.panel else ""
                shot = (
                    shots / f"{token}-{size[0]}x{size[1]}{panel}.png" if shots else None
                )
                # A store of its own per screen: the first visit is what a slow
                # link pays, and a warm store answers from its cache (measured
                # separately, below).
                screen_scratch = scratch / "screens" / f"{token}-{size[0]}x{size[1]}"
                screen_scratch.mkdir(parents=True, exist_ok=True)
                seed_session(app_url, screen_scratch, password=BENCH_PASSWORD)
                app = App(Path(args.binary), app_url, screen_scratch, display, bus.address)
                # The same X11 driver for every launch: a fresh connection.
                screen = drive.Screen(display)
                mode = ""
                if route == "library:favorites":
                    mode = "scroll"
                elif route == "releases":
                    # The page whose default view is the full list: 40 wheel
                    # notches over a few thousand releases is the scroll its
                    # frames are measured on. The wheel, not `Page suivante`,
                    # because the OLD page had no page keys — the same input has
                    # to scroll both builds for the two tables to compare.
                    mode = "wheel"
                elif route == "viz":
                    mode = "viz"
                try:
                    payload = measure_screen(
                        app,
                        screen,
                        proxy,
                        route,
                        size,
                        shot,
                        mode,
                        args.panel,
                        args.settle_ms,
                    )
                except Exception as error:  # a screen that fails is a finding
                    payload = {"route": route, "size": f"{size[0]}x{size[1]}",
                               "error": str(error)}
                    log("FAILED", route, error)
                results["screens"].append(payload)
                log(
                    f"{route} {size[0]}x{size[1]}",
                    payload.get("requests"),
                    "requests",
                    payload.get("bytes"),
                    "B",
                    payload.get("rss_kb"),
                    "kB RSS",
                )
                app.stop()
        # The app's own startup, measured once per size: `/api/me` and the
        # reachability probe, from a store that has never seen the library.
        for size in parse_sizes(args.sizes):
            if args.skip_screens:
                continue
            try:
                baseline_scratch = scratch / "baseline" / f"{size[0]}x{size[1]}"
                baseline_scratch.mkdir(parents=True, exist_ok=True)
                seed_session(app_url, baseline_scratch, password=BENCH_PASSWORD)
                app = App(
                    Path(args.binary), app_url, baseline_scratch, display, bus.address
                )
                screen = drive.Screen(display)
                payload = measure_screen(app, screen, proxy, "queue", size, None, "")
                payload["route"] = "_startup"
                results["screens"].append(payload)
                log(
                    "_startup",
                    size,
                    payload.get("requests"),
                    "requests",
                    payload.get("bytes"),
                    "B",
                )
                app.stop()
            except Exception as error:  # a baseline that fails is not the table
                log("_startup failed:", error)
    finally:
        if args.out:
            # Written even when a late step threw: a measurement is worth the
            # rows it already has.
            Path(args.out).write_text(json.dumps(results, indent=2))
        bus.stop()
        xvfb.stop()
        proxy.stop()
        server.stop()
    print(json.dumps(results, indent=2))
    return 0


class NullProxy:
    """`--skip-screens` and any run without the proxy still speaks the API."""

    def stats(self, reset: bool = False) -> dict:
        return {"requests": 0, "bytes": 0, "paths": {}}


def parse_sizes(text: str) -> list[tuple[int, int]]:
    sizes = []
    for part in text.split(","):
        part = part.strip()
        if not part:
            continue
        width, _, height = part.partition("x")
        sizes.append((int(width), int(height)))
    return sizes


def make_cover(scratch: Path) -> Path | None:
    """A real 1 000 px sleeve, because a 404 cover measures as "sleeves are free"."""
    path = scratch / "cover.jpg"
    if not path.exists():
        subprocess.run(
            [
                "convert",
                "-size",
                "1000x1000",
                "gradient:#2b6cb0-#c05621",
                "-blur",
                "0x8",
                str(path),
            ],
            check=True,
        )
    return path


if __name__ == "__main__":
    raise SystemExit(main())
