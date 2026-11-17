#!/usr/bin/env python3
"""One hour of listening, measured: 0 underruns while the machine is busy.

The D17 criterion is not "the sound works" (D5 proved the ring, the callback
and the seam); it is that nothing the app does in the background can starve the
output. So this script makes the app do everything at once for an hour:

* **plays** a 90-second FLAC, muted (`Ctrl+↓` × 20 before the first click, so
  the person's speakers stay silent), for the whole run;
* **downloads** 40 tracks of the 4 000-track playlist through `nz-bulk` — the
  app's own offline worker, resumed from the store exactly as after a restart
  (D3), so no UI click is needed and the Bulk lane's gates are the ones used;
* runs a **real transcription** next to it (`neezer-transcribe file` on a
  10-minute FLAC with a real ggml model, in a loop): the same binary the app
  spawns, the same whisper.cpp build, competing for CPU and Vulkan.

`--bench-status` writes the engine's own underrun counter every 10 s; the
report is the MAXIMUM over the hour, because the counter is monotonic. The run
fails (exit 1) on a single underrun, and says when it happened, which track was
playing and how the download queue stood.

    python desktop/tools/journey/soak.py --seconds 3600 \
        --model ~/.cache/neezer-d17/ggml-tiny.bin

Nothing here touches the person's X session: the window opens on the bench's
own Xvfb, and the sound goes through PipeWire at volume 0.
"""

from __future__ import annotations

import argparse
import json
import os
import signal
import sqlite3
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import bench  # noqa: E402
import drive  # noqa: E402


def seed_downloads(store: Path, owner: str, tracks: list[str], count: int) -> int:
    """Queue downloads the way the worker's resume path reads them.

    `downloads.pending(owner, 1)` is `state IN ('pending','active')`; the app
    calls it at start with `api.owner()`, so a row written here IS a download
    the person asked for before a restart. Quality is the app's default
    (`OPUS_320`).
    """
    database = store / "desktop.sqlite"
    added = 0
    stamp = int(time.time() * 1000)
    with sqlite3.connect(database, timeout=30) as connection:
        for index, track in enumerate(tracks[:count]):
            try:
                connection.execute(
                    "INSERT INTO downloads(owner, track, quality, file, bytes, state, error, "
                    "added_at) VALUES(?,?,?,?,?,?,?,?) "
                    "ON CONFLICT(owner, track) DO NOTHING",
                    (owner, track, "OPUS_320", None, None, "pending", None, stamp + index),
                )
                added += 1
            except sqlite3.Error as error:
                print("download row:", error, file=sys.stderr)
        connection.commit()
    return added


def owner_id(server_url: str, cookie: str) -> str:
    me = bench.api(server_url, "/api/me", cookie)
    return str(me["user"]["id"])


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--seconds", type=int, default=3600)
    parser.add_argument("--python", default=os.environ.get("NZ_PYTHON", "python3"))
    parser.add_argument("--binary", default=str(bench.DESKTOP / "target/release/neezer-desktop"))
    parser.add_argument("--transcribe", default=str(bench.DESKTOP / "target/release/neezer-transcribe"))
    parser.add_argument("--model", default=os.environ.get("NZ_WHISPER_MODEL", ""))
    parser.add_argument("--downloads", type=int, default=40)
    parser.add_argument("--scratch", default="")
    parser.add_argument("--out", default="")
    parser.add_argument("--display-base", type=int, default=130,
                        help="first X display this soak may take")
    parser.add_argument("--degraded", action="store_true",
                        help="run the whole soak behind throttle.py (300 ms, 1 Mb/s, 2 %%)")
    args = parser.parse_args()

    scratch = Path(args.scratch) if args.scratch else Path(tempfile.mkdtemp(prefix="nz-soak-"))
    scratch.mkdir(parents=True, exist_ok=True)
    server = bench.Server(args.python, bench.make_cover(scratch))
    server.start()
    print("server", server.url(), flush=True)
    proxy = None
    app_url = server.url()
    if args.degraded:
        proxy = bench.Proxy(f"127.0.0.1:{server.port}", scratch)
        proxy.start(latency_ms=150, rate_bps=1_000_000, loss=0.02)
        app_url = proxy.url()
    xvfb = bench.Xvfb(scratch)
    display = xvfb.start(base=args.display_base)
    bus = bench.start_bus()
    app = bench.App(Path(args.binary), app_url, scratch, display, bus.address)
    screen = drive.Screen(display)
    report: dict = {"server": server.url(), "degraded": bool(proxy), "seconds": args.seconds}
    transcribe: subprocess.Popen | None = None
    try:
        bench.seed_session(app_url, scratch)
        cookie = bench.login_cookie(server.url())
        owner = owner_id(server.url(), cookie)
        playlists = bench.api(server.url(), "/api/me/playlists", cookie)["playlists"]
        big = max(playlists, key=lambda row: row.get("track_count") or 0)
        rows = bench.api(server.url(), f"/api/playlist/{big['id']}", cookie)["tracks"]
        # The queue/download key is the ROW id (a uuid): `/api/stream` accepts
        # it through `canonical()`, and `downloads.track` holds the same value.
        tracks = [str(row["id"]) for row in rows]
        track_titles = {str(row["id"]): row["title"] for row in rows}
        # One real FLAC replaces the bench's silent WAV for the tracks the soak
        # plays and transcribes: the app decodes FLAC, and whisper needs audio
        # that is not a WAV.
        archive = server.workdir / "archive" / "journey"
        archive.mkdir(parents=True, exist_ok=True)
        music = scratch / "music"
        music.mkdir(parents=True, exist_ok=True)
        import shutil as _shutil

        flacs = [
            Path(_shutil.copyfile(path, archive / f"soak-{index}.flac"))
            for index, path in enumerate(bench.make_flacs(music, count=2, seconds=30))
        ]
        long_flac = Path(
            _shutil.copyfile(
                bench.make_flacs(music, count=1, seconds=600)[0], archive / "soak-long.flac"
            )
        )
        for track, flac in zip(tracks[:2], flacs):
            bench.point_track_at(server.workdir, track, flac)

        # First launch: it creates the store (and the session jar is already
        # there). Then quit, seed the download queue, and start the soak.
        app.start(f"playlist:{big['id']}", (1280, 800))
        if app.read_until(lambda line: line.startswith("bench logged_in"), 90) is None:
            raise RuntimeError("the first launch never signed in")
        app.stop()
        store = scratch / "xdg" / "data" / "neezer"
        queued = seed_downloads(store, owner, tracks[2:], args.downloads)
        print(f"{queued} downloads queued for owner {owner}", flush=True)

        for stale in (app.events, app.frames, app.status):
            if stale.exists():
                stale.write_text("")
        app.start(f"playlist:{big['id']}", (1280, 800))
        if app.read_until(lambda line: line.startswith("bench logged_in"), 90) is None:
            raise RuntimeError("the soak launch never signed in")
        window = screen.wait_window()
        screen.focus(window)
        # The rows come from `/api/playlist/<id>`; the keys reach an empty list
        # before they land (measured: a run that played nothing at all).
        bench.settle(proxy, quiet_ms=600.0, timeout=40.0) if proxy else time.sleep(2.0)
        # Volume to 0 BEFORE the first sound: the person's speakers stay silent
        # while the exact same callback path runs.
        for _ in range(20):
            screen.keys("ctrl-down")
        geometry = window.get_geometry()
        for _ in range(3):
            screen.click(geometry.width // 2 + 60, geometry.height // 2)
            time.sleep(0.4)
            screen.keys("home")
            screen.keys("enter")
            time.sleep(0.8)
            if any(row["event"] == "started" for row in app.events_log()):
                break
        # Let the whole 30 s body land so `StreamComplete` publishes it into the
        # DEVICE play cache, then play it from there with repeat one: the hour
        # is then a local file looping, and what the downloads and the
        # transcription compete with is decode + output, which is the criterion.
        # (Measured: playing the SERVER's generated transcode looped at ~29.5 s
        # per voice — the generated body is not the whole track — which made a
        # "continuous hour" a claim about a restart cycle.)
        first = next(
            (row.get("id") for row in app.events_log() if row["event"] == "load"), ""
        )
        deadline = time.monotonic() + 90
        while time.monotonic() < deadline:
            if any(
                row["event"] == "stream_complete" and row.get("id") == first
                for row in app.events_log()
            ):
                break
            time.sleep(0.5)
        screen.keys("home")
        screen.keys("enter")
        time.sleep(0.8)
        screen.keys("r")
        screen.keys("r")

        # The transcription, on the same machine, for the whole run.
        if args.model and Path(args.model).exists() and Path(args.transcribe).exists():
            out = scratch / "transcribe.json"
            log = open(scratch / "transcribe.log", "ab")
            transcribe = subprocess.Popen(
                [
                    "sh",
                    "-c",
                    f'while true; do "{args.transcribe}" file "{long_flac}" '
                    f'--model "{args.model}" --out "{out}" || true; done',
                ],
                stdout=log,
                stderr=log,
                # Rule 1: the transcription yields to the music, as the app's
                # supervisor does (`nice 19`, SCHED_IDLE). `nice` is a shell
                # builtin here because `Popen` has no `nice` argument.
                preexec_fn=lambda: os.nice(19),
                start_new_session=True,
            )
        else:
            print("no model: the transcription leg is skipped", file=sys.stderr)

        started = time.monotonic()
        deadline = started + args.seconds
        last_position = 0.0
        stalls = 0
        while time.monotonic() < deadline:
            time.sleep(30)
            lines = app.status.read_text().splitlines() if app.status.exists() else []
            last = json.loads(lines[-1]) if lines else {}
            position = last.get("position_s", 0.0)
            status_t = last.get("t_ms", 0)
            status = last.get("underruns", 0)
            print(
                f"{int(time.monotonic() - started):5d} s  position {position:7.2f} s  "
                f"underruns {status}  rss {last.get('rss_kb', 0)} kB",
                flush=True,
            )
            started_recently = any(
                row["event"] == "started" and row["t_ms"] + 60_000 > status_t
                for row in app.events_log()[-40:]
            )
            if position <= last_position + 0.1 and not started_recently:
                # The engine stopped for good (a row that cannot play, a stream
                # that never arrived): go back to row 0. `Entrée` alone re-jumps
                # to the SAME row, which is how the first soak sat silent for
                # 780 s. A `started` in the last minute means the queue moved on
                # and the position reset is a track change, not a stall.
                stalls += 1
                screen.keys("home")
                screen.keys("enter")
                time.sleep(0.5)
            last_position = position
        statuses = [
            json.loads(line)
            for line in (app.status.read_text().splitlines() if app.status.exists() else [])
            if line.strip()
        ]
        underruns = max((row.get("underruns", 0) for row in statuses), default=0)
        first = next(
            (row for row in statuses if row.get("underruns", 0) > 0), None
        )
        events = app.events_log()
        plus = [event for event in events if event["event"] == "stream_complete"]
        started_events = [event for event in events if event["event"] == "started"]
        report.update(
            {
                "status_lines": len(statuses),
                "underruns": underruns,
                "first_underrun": first,
                "stalls": stalls,
                "downloads_queued": queued,
                "downloads_observed": len(plus),
                "tracks_started": len(started_events),
                "rss_min_kb": min((row.get("rss_kb", 0) for row in statuses), default=0),
                "rss_max_kb": max((row.get("rss_kb", 0) for row in statuses), default=0),
                "last_status": statuses[-1] if statuses else None,
                "titles": {track: track_titles.get(track, "") for track in tracks[:2]},
            }
        )
    finally:
        if transcribe is not None:
            try:
                os.killpg(os.getpgid(transcribe.pid), signal.SIGTERM)
            except OSError:
                transcribe.terminate()
        app.stop()
        bus.stop()
        xvfb.stop()
        if proxy:
            proxy.stop()
        server.stop()
    print(json.dumps(report, indent=2))
    if args.out:
        Path(args.out).write_text(json.dumps(report, indent=2))
    return 0 if report.get("underruns", 1) == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
