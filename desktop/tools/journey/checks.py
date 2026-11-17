#!/usr/bin/env python3
"""The two D17 checks that are not a screenshot: click→sound, and MPRIS hidden.

**Click to sound**, in the three states a track can be in, on purpose:

* *cold* — the server has the FLAC on disk but has never transcoded it, so
  `/stream?q=OPUS_320` is generated as it is sent (`Accept-Ranges: none`): the
  first play of a finished archive.
* *archived* — the same track again, its Opus transcode now in the server's
  cache, served as a finished file with a length.
* *cached* — a track the device already holds in its play cache (`nz-cache`):
  a six-second FLAC played to its end once, so `StreamComplete` published it;
  the second click asks the network for nothing.

The click is a real X11 click on the list (`drive.Screen.click`), and the
timing is the app's own: `--bench-events` writes `load` when the gesture opens
the stream and `started` when the engine's first sample reaches the mixer
(`EngineEvent::Started`), so `started - load` is the number, measured without a
stopwatch in the loop.

**MPRIS with the window hidden**: D7 reported that the 4 Hz position froze when
the window was not visible — a minimised window or another Hyprland workspace
stops GPUI's frame loop, and the tick lived in it. The app now publishes the
position from its own thread (`wiring::mpris::PositionClock`). This check reads
`playerctl position` twice with the window on screen, then unmaps the window
(`drive.Screen.hide`, which is what a workspace switch does to this app) and
reads it twice again. Both pairs must advance.

    python desktop/tools/journey/checks.py --out /tmp/nz-checks.json

Same isolation as `bench.py`: our own Xvfb (`:130`+), our own D-Bus, our own
XDG tree, and `WAYLAND_DISPLAY` stripped so GPUI cannot use the person's
session.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import bench  # noqa: E402
import drive  # noqa: E402


def bench_settle(proxy: "bench.Proxy", quiet_ms: float = 600.0) -> dict:
    return bench.settle(proxy, quiet_ms=quiet_ms, timeout=40.0)


def playerctl(bus: str, *args: str) -> str:
    env = dict(os.environ, DBUS_SESSION_BUS_ADDRESS=bus)
    env.pop("WAYLAND_DISPLAY", None)
    result = subprocess.run(
        ["playerctl", "-p", "neezer", *args], capture_output=True, text=True, env=env
    )
    return result.stdout.strip()


def position(bus: str) -> float:
    try:
        return float(playerctl(bus, "position"))
    except ValueError:
        return 0.0


def wait_started(events: dict[str, list[dict]], track: str, after: float, timeout: float = 30.0):
    """The first `started` for `track` logged after `after` ms."""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        rows = events.get("rows", [])
        starts = [
            row
            for row in rows
            if row["event"] == "started" and row.get("id") == track and row["t_ms"] >= after
        ]
        if starts:
            return starts[0]
        time.sleep(0.05)
    return None


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--python", default=os.environ.get("NZ_PYTHON", "python3"))
    parser.add_argument("--binary", default=str(bench.DESKTOP / "target/release/neezer-desktop"))
    parser.add_argument("--scratch", default="")
    parser.add_argument("--degraded", action="store_true",
                        help="ask the server through throttle.py (300 ms, 1 Mb/s, 2 %%)")
    parser.add_argument("--out", default="")
    args = parser.parse_args()

    scratch = Path(args.scratch) if args.scratch else Path(tempfile.mkdtemp(prefix="nz-checks-"))
    scratch.mkdir(parents=True, exist_ok=True)
    server = bench.Server(args.python, bench.make_cover(scratch))
    server.start()
    proxy = bench.Proxy(f"127.0.0.1:{server.port}", scratch)
    if args.degraded:
        proxy.start(latency_ms=150, rate_bps=1_000_000, loss=0.02)
    else:
        proxy.start(latency_ms=0.0, rate_bps=0, loss=0.0)
    xvfb = bench.Xvfb(scratch)
    display = xvfb.start()
    bus = bench.start_bus()
    app = bench.App(Path(args.binary), proxy.url(), scratch, display, bus.address)
    screen = drive.Screen(display)
    report: dict = {"server": server.url(), "app": proxy.url()}
    try:
        bench.seed_session(proxy.url(), scratch)
        cookie = bench.login_cookie(server.url())
        playlists = bench.api(server.url(), "/api/me/playlists", cookie)["playlists"]
        big = max(playlists, key=lambda row: row.get("track_count") or 0)
        rows = bench.api(server.url(), f"/api/playlist/{big['id']}", cookie)["tracks"]
        # The queue's key is the ROW id (a uuid); the stream route accepts it
        # (`canonical()` maps it to the Deezer id). The DB row is keyed by it
        # too — a `deezer_id` there would update nothing and the stream would
        # answer 503 "not archived".
        tracks = [str(row["id"]) for row in rows]
        titles = {str(row["id"]): row["title"] for row in rows}
        # A six-second FLAC so a whole play fits in the play cache, and a
        # ninety-second one for the cold/archived pair.
        # The fixtures live INSIDE the archive: the server serves only what is
        # under it, so a row pointed elsewhere answers 503 and never plays.
        archive = server.workdir / "archive" / "journey"
        archive.mkdir(parents=True, exist_ok=True)
        scratch_music = scratch / "music"
        scratch_music.mkdir(parents=True, exist_ok=True)
        short = bench.make_flacs(scratch_music, count=1, seconds=6)[0]
        long = bench.make_flacs(scratch_music, count=3, seconds=90)[1]
        import shutil as _shutil

        short = Path(_shutil.copyfile(short, archive / "short.flac"))
        long = Path(_shutil.copyfile(long, archive / "long.flac"))
        # Rows 0, 1 (the prefetch after row 0) and 5 (never prefetched: the
        # app preloads the row AFTER the playing one, so a click on row 5 is a
        # real cold start).
        cold = tracks[5]
        bench.point_track_at(server.workdir, tracks[0], short)
        bench.point_track_at(server.workdir, tracks[1], long)
        bench.point_track_at(server.workdir, cold, long)


        def observe(label: str, steps: list[str], track: str) -> dict:
            before = app.events_log()
            after = before[-1]["t_ms"] if before else 0
            proxy.stats(reset=True)
            for step in steps:
                screen.keys(step)
                time.sleep(0.35)
            events = {"rows": app.events_log()}
            # A FRESH `load` for this track after the gesture, then the first
            # `started` for the same track after it. Without the load check a
            # replayed row that never reloads (the engine emits `Started` once
            # per voice) matches the previous play's event and reports a 6 ms
            # click-to-sound that never happened.
            loads = [
                row
                for row in events["rows"]
                if row["event"] == "load" and row.get("id") == track and row["t_ms"] > after
            ]
            started = None
            if loads:
                started = wait_started(events, track, loads[-1]["t_ms"] - 1)
            snapshot = proxy.stats()
            row = {
                "state": label,
                "track": track,
                "title": titles.get(track, ""),
                "requests": snapshot["requests"],
                "bytes": snapshot["bytes"],
                "paths": snapshot.get("paths", {}),
            }
            if started is not None:
                row["load_ms"] = loads[-1]["t_ms"]
                row["started_ms"] = started["t_ms"]
                row["click_to_sound_ms"] = started.get("ms")
            elif loads:
                row["load_ms"] = loads[-1]["t_ms"]
                row["error"] = "the stream opened but the engine never started this track"
            else:
                row["error"] = "the gesture did not load this track (no fresh load event)"
            print(label, json.dumps(row, ensure_ascii=False)[:220], flush=True)
            return row

        played = []

        def run_screen(store: Path, label: str, track: str, keep: bool = False):
            """One app run from `store`, one click on row 0, one measurement."""
            store.mkdir(parents=True, exist_ok=True)
            bench.seed_session(proxy.url(), store)
            app = bench.App(Path(args.binary), proxy.url(), store, display, bus.address)
            driver = drive.Screen(display)
            proxy.stats(reset=True)
            app.start(f"playlist:{big['id']}", (1280, 800))
            if app.read_until(lambda line: line.startswith("bench logged_in"), 120) is None:
                raise RuntimeError("the session was never restored")
            win = driver.wait_window()
            driver.focus(win)
            # The playlist's rows come from `/api/playlist/<id>`: pressing the
            # keys before they land reaches an EMPTY list (`Intent::PlayFrom`
            # plays nothing, measured as a run with no `load` at all). Wait for
            # the link to go quiet first.
            bench_settle(proxy)
            geometry = win.get_geometry()
            before = app.events_log()
            after = before[-1]["t_ms"] if before else 0
            loads = []
            for _ in range(3):
                driver.click(geometry.width // 2 + 60, geometry.height // 2)
                time.sleep(0.4)
                driver.keys("home")
                driver.keys("enter")
                time.sleep(0.8)
                rows = app.events_log()
                loads = [
                    row
                    for row in rows
                    if row["event"] == "load"
                    and row.get("id") == track
                    and row["t_ms"] > after
                ]
                if loads:
                    break
            started = None
            if loads:
                deadline = time.monotonic() + 30
                while time.monotonic() < deadline and started is None:
                    for row in app.events_log():
                        if (
                            row["event"] == "started"
                            and row.get("id") == track
                            and row["t_ms"] >= loads[-1]["t_ms"]
                        ):
                            started = row
                            break
                    time.sleep(0.05)
            snapshot = proxy.stats()
            result = {
                "state": label,
                "track": track,
                "title": titles.get(track, ""),
                "requests": snapshot["requests"],
                "bytes": snapshot["bytes"],
                "paths": snapshot.get("paths", {}),
            }
            if started is not None:
                result["load_ms"] = loads[-1]["t_ms"]
                result["started_ms"] = started["t_ms"]
                result["click_to_sound_ms"] = started.get("ms")
            elif loads:
                result["error"] = "the stream opened but the engine never started this track"
            else:
                result["error"] = "the gesture did not load this track"
            if keep:
                return result, app, driver, win
            app.stop()
            return result

        # The three states, each from a store chosen for what it holds — and
        # with row 0 as the target, because the app prefetches the row AFTER
        # the playing one and would otherwise have fetched the target already.
        stores = scratch / "stores"
        cold, cold_app, cold_screen, cold_window = run_screen(
            stores / "cold", "cold", tracks[0], keep=True
        )
        played.append(cold)
        print("cold", json.dumps(cold, ensure_ascii=False)[:200], flush=True)
        # Let the whole body land so `StreamComplete` publishes it into the
        # play cache — the app must still be RUNNING for that (it stopped after
        # the sound started otherwise, and the store copy had no cache).
        time.sleep(12.0)
        cold_app.stop()
        import shutil as _shutil

        cached_store = stores / "cached"
        if cached_store.exists():
            _shutil.rmtree(cached_store)
        _shutil.copytree(stores / "cold", cached_store)
        warm = run_screen(stores / "archived", "archived", tracks[0])
        played.append(warm)
        print("archived", json.dumps(warm, ensure_ascii=False)[:200], flush=True)
        result, app, screen, window = run_screen(
            cached_store, "cached", tracks[0], keep=True
        )
        played.append(result)
        print("cached", json.dumps(result, ensure_ascii=False)[:200], flush=True)
        report["click"] = played

        # --- MPRIS with the window on screen, then hidden --------------------
        # Restart row 0 and read at once: the six-second track must not end
        # between two reads (measured: the position *reset* to 0.4 s, which
        # reads as a 4 s regression and is a track change, not a freeze).
        screen.keys("home")
        screen.keys("enter")
        time.sleep(0.35)
        visible_start = position(bus.address)
        time.sleep(2.0)
        visible = (visible_start, position(bus.address))
        screen.hide(window)
        time.sleep(0.5)
        hidden_start = position(bus.address)
        time.sleep(2.0)
        hidden = (hidden_start, position(bus.address))
        hidden_viewable = screen.is_viewable(window)
        screen.show(window)
        report["mpris"] = {
            "visible_positions": visible,
            "visible_advance_s": round(visible[1] - visible[0], 2),
            "hidden_positions": hidden,
            "hidden_viewable": hidden_viewable,
            "hidden_advance_s": round(hidden[1] - hidden[0], 2),
        }
        print("mpris", report["mpris"], flush=True)
    finally:
        app.stop()
        bus.stop()
        xvfb.stop()
        proxy.stop()
        server.stop()

    if args.out:
        Path(args.out).write_text(json.dumps(report, indent=2))
    print(json.dumps(report, indent=2))
    ok = (
        report.get("mpris", {}).get("visible_advance_s", 0) > 1.0
        and report.get("mpris", {}).get("hidden_advance_s", 0) > 1.0
        and all(row.get("click_to_sound_ms") is not None for row in report.get("click", []))
    )
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
