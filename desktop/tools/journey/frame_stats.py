#!/usr/bin/env python3
"""Frame times from a `neezer-desktop --frame-log` file.

    python desktop/tools/journey/frame_stats.py frames.log
    python desktop/tools/journey/frame_stats.py frames.log --phases 8:38,45:75,82:112

Each line is `frame <t_ms> <build_us>`: when the app painted, and how long the
element tree took to build. The DESCENDING intervals are what the panel
actually delivered (`fps`), and they are the only honest frame-time number: a
paint that happens because a key arrived is not a frame rate. Intervals longer
than a quarter second are counted separately as `gaps` (the app was idle, not
slow), so a scroll or an animation is measured over its own 30 s window and an
idle app does not dilute it.

`--phases a:b,c:d` reports one row per window in the app's own milliseconds,
which is what `frame_bench.sh` asks the person to do on their real display:
navigate, scroll, then open the visualiser, 30 s each.
"""

from __future__ import annotations

import argparse
import statistics
import sys
from pathlib import Path


def read_frames(path: Path) -> list[tuple[int, float]]:
    rows = []
    for line in path.read_text().splitlines():
        parts = line.split()
        if len(parts) != 3 or parts[0] != "frame":
            continue
        try:
            rows.append((int(parts[1]), int(parts[2]) / 1000.0))
        except ValueError:
            continue
    return rows


def percentile(values: list[float], fraction: float) -> float:
    if not values:
        return 0.0
    ordered = sorted(values)
    index = min(len(ordered) - 1, max(0, round(fraction / 100 * (len(ordered) - 1))))
    return ordered[index]


def summarise(rows: list[tuple[int, float]], label: str) -> dict:
    intervals = [b[0] - a[0] for a, b in zip(rows, rows[1:])]
    continuous = [value for value in intervals if value <= 250]
    gaps = len(intervals) - len(continuous)
    builds = [build for _, build in rows]
    sample = continuous or intervals
    summary = {
        "label": label,
        "frames": len(rows),
        "fps": round(1000.0 / statistics.fmean(sample), 1) if sample else 0.0,
        "p50_ms": round(percentile(sample, 50), 2),
        "p95_ms": round(percentile(sample, 95), 2),
        "max_ms": round(max(sample), 2) if sample else 0.0,
        "gaps": gaps,
        "build_p50_ms": round(percentile(builds, 50), 3),
        "build_p95_ms": round(percentile(builds, 95), 3),
        "build_max_ms": round(max(builds), 3) if builds else 0.0,
    }
    return summary


def print_table(summaries: list[dict]) -> None:
    print(
        f"{'phase':<22}{'frames':>8}{'fps':>7}{'p50 ms':>9}{'p95 ms':>9}"
        f"{'max ms':>9}{'gaps':>7}{'build p95':>11}"
    )
    for row in summaries:
        print(
            f"{row['label']:<22}{row['frames']:>8}{row['fps']:>7}"
            f"{row['p50_ms']:>9}{row['p95_ms']:>9}{row['max_ms']:>9}"
            f"{row['gaps']:>7}{row['build_p95_ms']:>11}"
        )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("log", type=Path)
    parser.add_argument(
        "--phases",
        default="",
        help="comma-separated `start:end` windows in the app's own ms",
    )
    parser.add_argument("--json", default="", help="write the table here as JSON")
    args = parser.parse_args()

    rows = read_frames(args.log)
    if not rows:
        print(f"no frame in {args.log}", file=sys.stderr)
        return 1
    summaries = []
    if args.phases:
        for phase in args.phases.split(","):
            start_text, _, end_text = phase.partition(":")
            start, end = int(start_text), int(end_text)
            window = [row for row in rows if start <= row[0] < end]
            summaries.append(summarise(window, f"{start / 1000:.0f}–{end / 1000:.0f} s"))
    else:
        summaries.append(summarise(rows, "whole run"))
    print_table(summaries)
    if args.json:
        import json

        Path(args.json).write_text(json.dumps(summaries, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
