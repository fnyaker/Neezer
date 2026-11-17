#!/usr/bin/env python3
"""Turn the Macrobenchmark report into the small JSON `measure.sh` writes.

AndroidX writes `<test>-benchmarkData.json` on the device (`additional_test_output`
under `/sdcard/Android/media/<package>/`) and Gradle copies it into
`build/outputs/connected_android_test_additional_output/...`; its shape is one
`benchmarks[]` entry per test with `metrics`/`sampledMetrics` objects holding
`minimum`, `maximum`, `median` and the per-iteration `runs`.

That file is 200 kB of context, device description and duplicated run times; what
a lot writes in its PR, and what the next run is compared against, is the median
and the tail percentiles of each metric. This script keeps the second and drops
the first, and it FAILS (exit 1) when a report holds no metric at all: a bench
that passed without measuring anything is the exact failure mode MQ1 exists to
catch (the emulator's `-no-window` frame source, for instance, is a real risk).

Usage:
  benchdata.py --what startup --serial emulator-5580 --out report.json <benchmarkData.json>...
"""

from __future__ import annotations

import argparse
import json
import math
import sys
from datetime import datetime, timezone
from pathlib import Path

# The metrics a report must carry for each kind of bench to be worth anything.
EXPECTED = {
    "startup": ("timeToInitialDisplayMs",),
    "frames": ("frameDurationCpuMs",),
    "profile": (),  # the generator measures nothing; it only produces a profile
}


def percentile(sorted_values: list[float], pct: float) -> float:
    """The nearest-rank percentile of an already sorted list.

    Nearest rank, not an interpolation: with 10 runs an interpolated p95 invents a
    number between two frames that were actually drawn, and the plan's "p95 ≤
    16.7 ms" is a statement about frames, not about a curve fitted to them.
    """
    if not sorted_values:
        return math.nan
    rank = max(1, math.ceil(pct / 100 * len(sorted_values)))
    return sorted_values[min(rank, len(sorted_values)) - 1]


def flatten_runs(runs: list) -> list[float]:
    """The per-iteration values, flattened.

    `metrics.<name>.runs` holds one number per iteration, but a SAMPLED metric
    (`FrameTimingMetric`'s per-frame durations) holds one LIST per iteration — one
    entry per frame drawn. Flattening is what lets the percentiles be computed
    over frames: p95 ≤ 16.7 ms is a statement about frames, so pooling them is
    the correct population, not the median of five medians.
    """
    flat: list[float] = []
    for value in runs:
        if isinstance(value, list):
            flat.extend(float(v) for v in value)
        else:
            flat.append(float(value))
    return flat


def summarise_metric(name: str, body: dict) -> dict:
    values = sorted(flatten_runs(body.get("runs") or []))
    out = {
        "unit": body.get("unit") or ("ms" if name.endswith("Ms") else ""),
        "samples": len(values),
        "median": body.get("median", percentile(values, 50)),
        "min": body.get("minimum"),
        "max": body.get("maximum"),
    }
    if values:
        out.update(
            {
                "p50": percentile(values, 50),
                "p90": percentile(values, 90),
                "p95": percentile(values, 95),
                "p99": percentile(values, 99),
            }
        )
    return out


def summarise_report(path: Path) -> list[dict]:
    data = json.loads(path.read_text())
    out = []
    for bench in data.get("benchmarks", []):
        metrics: dict[str, dict] = {}
        for name, body in (bench.get("metrics") or {}).items():
            metrics[name] = summarise_metric(name, body)
        for name, body in (bench.get("sampledMetrics") or {}).items():
            metrics[name] = summarise_metric(name, body)
        out.append(
            {
                "name": bench.get("name"),
                "class": bench.get("className"),
                "iterations": bench.get("repeatIterations"),
                "metrics": metrics,
            }
        )
    return out


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--what", required=True, choices=sorted(EXPECTED))
    parser.add_argument("--serial", default="")
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--meta", action="append", default=[], metavar="KEY=VALUE")
    parser.add_argument("reports", nargs="+", type=Path)
    args = parser.parse_args()

    benchmarks: list[dict] = []
    for report in args.reports:
        if report.is_file():
            benchmarks.extend(summarise_report(report))

    if not benchmarks:
        print(f"benchdata: no benchmark entry in {[str(p) for p in args.reports]}", file=sys.stderr)
        return 1

    expected = EXPECTED[args.what]
    for bench in benchmarks:
        if expected and not any(m in bench["metrics"] for m in expected):
            print(
                f"benchdata: {bench['name']} measured none of {expected}: {sorted(bench['metrics'])}",
                file=sys.stderr,
            )
            return 1

    meta = {}
    for item in args.meta:
        key, _, value = item.partition("=")
        meta[key] = value

    payload = {
        "tool": "measure.sh",
        "what": args.what,
        "utc": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "serial": args.serial,
        **meta,
        "reports": [str(p) for p in args.reports if p.is_file()],
        "benchmarks": benchmarks,
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(payload, indent=2, sort_keys=False) + "\n")

    # A one-line human summary, so a CI log says the numbers without a JSON tool.
    for bench in benchmarks:
        bits = []
        for name, m in bench["metrics"].items():
            if "p95" in m:
                # The metric's OWN unit: `frameCount` is frames, not ms, and a
                # summary that prints "150.0 ms of frames" is a number nobody
                # should have to argue with.
                bits.append(
                    f"{name} p50={m['p50']:.1f} p95={m['p95']:.1f} p99={m['p99']:.1f} {m.get('unit') or ''}".strip()
                )
            elif m.get("median") is not None:
                bits.append(f"{name} median={m['median']:.1f} ms (n={m['samples']})")
        print(f"{bench['name']}: {', '.join(bits)}")
    print(f"json: {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
