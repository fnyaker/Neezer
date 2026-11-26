#!/usr/bin/env python3
"""The nightly performance gate: compare the Macrobenchmark reports with a baseline.

`mobile.yaml`'s `perf` job runs the four measures of *Measures* on the headless
emulator once a night (never on a pull request: a timing bound on an emulator is
skipped, not widened — docs/agents/mobile.md) and this script decides whether the
run got worse. It reads AndroidX's `*-benchmarkData.json` files through
`benchdata.py` (the same parser `measure.sh` uses, so a number here and a number
in a PR are the same number), takes the POOLED p95 of each benchmark, and fails
when that p95 is more than 10 % above the baseline's.

Four gates, one benchmark each (the names are the plan's; the test that measures
it is named beside it because the benches were renamed once already):

| gate | benchmark | metric |
|---|---|---|
| `startup` | `StartupBenchmark#startupColdWithoutProfile` | `timeToInitialDisplayMs` |
| `libraryScroll8000` | `JourneyListsBenchmark#favoritesScroll8000` | `frameDurationCpuMs` |
| `queueScroll4000` | `JourneyListsBenchmark#playlistScroll4000` | `frameDurationCpuMs` |
| `playerOpen` | `JourneyListsBenchmark#playerOpen` | `frameDurationCpuMs` |

`ScrollBenchmark`'s older `libraryScroll8000`/`queueScroll4000` are the MB0
skeleton's, written before the screens existed and skipped while their tab paints
no rows (`ScrollBenchmark.kt`'s `assumeTrue`); `JourneyListsBenchmark` is the
measure of the real screens (docs/agents/mobile.md, MQ2), so the gate reads those.

p95 and not the median: the owner's "every click is fast" is about the worst
frames a person feels, and the plan's budget (p95 ≤ 16.7 ms) is written on p95.
A regression of the tail is exactly what a scroll feels like.

Why a fingerprint. The numbers are the EMULATOR's: an APK's p95 on this machine
is comparable with another run on the same image and shape, never with a phone
budget (the plan's rule). The baseline carries the device it was recorded on, and
a report from another device is reported `incomparable` — NOT failed, because a
different runner is not a regression. The CPU model is printed when it differs
(the runner fleet is not homogeneous): the delta is then a hint, not a verdict.

Usage:
  perf-gate.py                        # compare every report under the module with the baseline
  perf-gate.py --record              # write the baseline from these reports
  perf-gate.py --self-test           # prove the gate fails an 11 % regression, passes 9 %
  perf-gate.py <report.json>...      # compare exactly these files
"""

from __future__ import annotations

import argparse
import contextlib
import glob
import json
import os
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path

HERE = Path(__file__).resolve().parent

# benchdata.py is the parser every number of the measure goes through; importing
# it (rather than a second copy) is what keeps the gate and a PR's figures equal.
sys.path.insert(0, str(HERE))
import benchdata  # noqa: E402

TOOL = "perf-gate.py"
THRESHOLD_PCT = 10.0
HERE_BASELINE = HERE / "baselines" / "perf.json"


@dataclass(frozen=True)
class Gate:
    """One budget: a benchmark's metric, held to a p95 in the baseline."""

    name: str
    benchmark: str  # the test's simple name in benchmarkData.json
    metric: str
    why: str


GATES: tuple[Gate, ...] = (
    Gate(
        "startup",
        "startupColdWithoutProfile",
        "timeToInitialDisplayMs",
        "cold start, the first frame a person waits for",
    ),
    Gate(
        "libraryScroll8000",
        "favoritesScroll8000",
        "frameDurationCpuMs",
        "the 8 000-favourite list, the worst realistic input",
    ),
    Gate(
        "queueScroll4000",
        "playlistScroll4000",
        "frameDurationCpuMs",
        "the 4 000-row queue, the worst realistic input",
    ),
    Gate(
        "playerOpen",
        "playerOpen",
        "frameDurationCpuMs",
        "the full-screen player's first frame",
    ),
)


# --- the reports ------------------------------------------------------------


def default_reports() -> list[Path]:
    """Every benchmarkData.json the `perf` job's Gradle task wrote.

    AndroidX leaves them under the module's `build/outputs` (Gradle copies the
    device's `additional_test_output` there); the glob is recursive because the
    directory gained a variant level between AGP versions.
    """
    root = HERE.parent / "android" / "baselineprofile" / "build" / "outputs"
    return sorted(Path(p) for p in glob.glob(str(root / "**" / "*-benchmarkData.json"), recursive=True))


def read_json(path: Path) -> dict:
    return json.loads(path.read_text())


def load(reports: list[Path]) -> tuple[dict, list[dict]]:
    """The reports' benchmarks, and the context of the first that has one.

    Two files are normal (one per `-benchmarkData.json` Gradle writes when a run
    is split); the context is the DEVICE's, so the first is the run's.
    """
    benchmarks: list[dict] = []
    context: dict = {}
    missing: list[str] = []
    for report in reports:
        if not report.is_file():
            missing.append(str(report))
            continue
        data = read_json(report)
        if not context:
            context = data.get("context") or {}
        benchmarks.extend(benchdata.summarise_report(report))
    if missing:
        raise SystemExit(f"{TOOL}: no report at {', '.join(missing)}")
    if not benchmarks:
        raise SystemExit(
            f"{TOOL}: no benchmark entry in {', '.join(str(r) for r in reports)} "
            "(the Gradle task wrote nothing: did the instrumented run fail?)"
        )
    return context, benchmarks


def summarise_device(context: dict) -> dict:
    """The part of the report's context that identifies the measuring device.

    Read defensively: the keys moved between Macrobenchmark versions, and a
    missing one must degrade to a smaller fingerprint, never crash the gate.
    """
    build = context.get("build") or {}
    version = build.get("version") or {}
    sdk = version.get("sdk") or build.get("sdk") or context.get("sdk")
    device = {
        "fingerprint": build.get("fingerprint") or build.get("model") or build.get("device"),
        "model": build.get("model"),
        "sdk": sdk,
        "abi": build.get("abi") or context.get("abi"),
        "cpu_core_count": context.get("cpuCoreCount"),
        "cpu_name": context.get("cpuName"),
    }
    return {k: v for k, v in device.items() if v is not None}


def fingerprint_of(device: dict) -> str:
    """What makes two runs comparable: the emulator image and its shape.

    Deliberately NOT the CPU model: the runner fleet is heterogeneous and a gate
    that is `incomparable` every other night guards nothing. The model is printed
    when it differs, so a suspicious delta can be read for what it is.
    """
    parts = [
        str(device.get("fingerprint") or device.get("model") or "unknown-device"),
        f"sdk{device['sdk']}" if device.get("sdk") is not None else "sdk?",
        str(device.get("abi") or "abi?"),
        f"cores{device['cpu_core_count']}" if device.get("cpu_core_count") is not None else "cores?",
    ]
    return "/".join(parts)


def percentile_of(metric: dict, key: str) -> float | None:
    value = metric.get(key)
    return float(value) if isinstance(value, (int, float)) else None


def measure(benchmarks: list[dict], gate: Gate) -> dict | None:
    """The gate's numbers from the reports, or None when the test did not run.

    A test that ran and measured nothing is NOT the same as a missing test: the
    first means the bench drew no frame (the emulator's `-no-window` source, an
    empty screen), and `benchdata.py` already fails a report that carries no
    metric of its kind. Here the metric is named, so the message names the gate.
    """
    for bench in benchmarks:
        if (bench.get("name") or "") != gate.benchmark:
            continue
        metrics = bench.get("metrics") or {}
        body = metrics.get(gate.metric)
        if body is None:
            print(
                f"{TOOL}: {gate.benchmark} measured none of its metrics "
                f"{gate.metric} ({sorted(metrics)}): the screen painted no frame",
                file=sys.stderr,
            )
            return {"error": f"{gate.benchmark} measured none of its metrics {gate.metric}"}
        out = {
            "benchmark": gate.benchmark,
            "metric": gate.metric,
            "unit": body.get("unit") or "ms",
            "samples": body.get("samples") or 0,
            "p50": percentile_of(body, "p50"),
            "p95": percentile_of(body, "p95"),
            "p99": percentile_of(body, "p99"),
            "iterations": bench.get("iterations"),
        }
        if out["p95"] is None:
            print(
                f"{TOOL}: {gate.benchmark} carries no p95 for {gate.metric} (no run samples)",
                file=sys.stderr,
            )
            return {"error": f"{gate.benchmark} carries no p95 for {gate.metric}"}
        return out
    print(f"{TOOL}: {gate.benchmark} is not in the reports (it did not run)", file=sys.stderr)
    return {"error": f"{gate.benchmark} did not run"}


# --- the decision -----------------------------------------------------------


def fmt(value: float | None) -> str:
    return "—" if value is None else f"{value:.1f}"


def compare(
    reports: tuple[dict, list[dict]],
    baseline: dict,
    threshold: float = THRESHOLD_PCT,
) -> int:
    """Print the run beside the baseline; 1 on a regression, 0 otherwise."""
    context, benchmarks = reports
    device = summarise_device(context)
    fingerprint = fingerprint_of(device)
    base_device = baseline.get("device") or {}
    base_fingerprint = base_device.get("fingerprint") or fingerprint_of(base_device)

    if fingerprint != base_fingerprint:
        for gate in GATES:
            got = measure(benchmarks, gate)
            if got and "error" not in got:
                print(
                    f"{gate.name:<18} p50={fmt(got['p50'])} p95={fmt(got['p95'])} "
                    f"p99={fmt(got['p99'])} {got['unit']} (baseline was recorded on another device)"
                )
        print(f"{TOOL}: incomparable — this run's device is {fingerprint}, the baseline's {base_fingerprint}")
        print(
            "perf_gate=incomparable "
            f"device={fingerprint} baseline_device={base_fingerprint}"
        )
        return 0

    if device.get("cpu_name") and base_device.get("cpu_name"):
        if device["cpu_name"] != base_device["cpu_name"]:
            print(
                f"{TOOL}: note — the host CPU differs from the baseline's "
                f"({device['cpu_name']} vs {base_device['cpu_name']}): part of any delta is the host's",
                file=sys.stderr,
            )

    failures: list[str] = []
    print(f"{'gate':<18} {'metric':<24} {'p50':>8} {'p95':>8} {'p99':>8}  baseline p50/p95   delta p95")
    for gate in GATES:
        base = (baseline.get("gates") or {}).get(gate.name)
        if not base:
            failures.append(f"{gate.name}: the baseline carries no entry for it (re-record with --record)")
            print(f"{gate.name:<18} MISSING FROM THE BASELINE")
            continue
        got = measure(benchmarks, gate)
        if got is None or "error" in got:
            failures.append(f"{gate.name}: {got['error'] if got else 'not measured'}")
            print(f"{gate.name:<18} NOT MEASURED")
            continue

        base_p95 = float(base["p95"])
        delta_pct = (got["p95"] - base_p95) / base_p95 * 100.0 if base_p95 else 0.0
        verdict = "ok"
        if delta_pct > threshold:
            verdict = f"REGRESSION (+{delta_pct:.1f} % > {threshold:g} %)"
            failures.append(
                f"{gate.name} p95 {got['p95']:.1f} {got['unit']} vs baseline {base_p95:.1f} {got['unit']} "
                f"(+{delta_pct:.1f} % > {threshold:g} %)"
            )
        else:
            verdict = f"ok ({delta_pct:+.1f} %)"
        print(
            f"{gate.name:<18} {got['metric']:<24} {fmt(got['p50']):>8} {fmt(got['p95']):>8} "
            f"{fmt(got['p99']):>8}  {fmt(base.get('p50'))}/{fmt(base_p95):<9} {verdict}"
        )

    if failures:
        for failure in failures:
            print(f"{TOOL}: {failure}", file=sys.stderr)
        print(f"perf_gate=fail regressions={len(failures)} threshold={threshold:g}")
        return 1
    print(f"perf_gate=ok gates={len(GATES)} threshold={threshold:g} device={fingerprint}")
    return 0


def record(reports: tuple[dict, list[dict]], commit: str, threshold: float, label: str = "") -> dict:
    """The baseline this run becomes: its header, its device, its four gates."""
    context, benchmarks = reports
    device = summarise_device(context)
    if label:
        device["label"] = label
    gates: dict[str, dict] = {}
    for gate in GATES:
        got = measure(benchmarks, gate)
        if got is None or "error" in got:
            raise SystemExit(
                f"{TOOL}: refusing to record a baseline without {gate.name} "
                f"({got['error'] if got else 'not measured'}): a gate with no baseline number guards nothing"
            )
        gates[gate.name] = {
            "benchmark": gate.benchmark,
            "metric": got["metric"],
            "unit": got["unit"],
            "samples": got["samples"],
            "iterations": got["iterations"],
            "p50": got["p50"],
            "p95": got["p95"],
            "p99": got["p99"],
        }
    return {
        "tool": TOOL,
        "recorded_utc": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "commit": commit,
        "thresholds": {"p95_regression_pct": threshold},
        "device": {**device, "fingerprint": fingerprint_of(device)},
        "gates": gates,
    }


# --- the gate's own proof ---------------------------------------------------


SYNTHETIC_RUNS = [100.0 + i for i in range(20)]


def synthetic_report(scale: dict[str, float], fingerprint: str = "test/emulator/35") -> Path:
    """A report with the four benchmarks, each metric scaled by its gate's factor.

    20 samples and a nearest-rank p95 that lands on the 19th value (118.0), so a
    factor is exactly the p95 delta the gate must read: 1.09 is +9 %, 1.11 is
    +11 %. Nothing here is a measurement — it is the gate's arithmetic, held to a
    number a person can check by hand.
    """
    b = []
    for gate in GATES:
        runs = [round(v * scale.get(gate.name, 1.0), 4) for v in SYNTHETIC_RUNS]
        body = {
            "unit": "ms",
            "minimum": min(runs),
            "maximum": max(runs),
            "median": runs[len(runs) // 2],
            "runs": runs,
        }
        b.append(
            {
                "name": gate.benchmark,
                "className": f"fr.neezer.mobile.baselineprofile.Synthetic#{gate.benchmark}",
                "params": {},
                "repeatIterations": len(runs),
                "metrics": {gate.metric: body},
                "sampledMetrics": {},
            }
        )
    payload = {
        "context": {
            "build": {"fingerprint": fingerprint, "model": "sdk_gphone64_x86_64", "version": {"sdk": 35}},
            "cpuCoreCount": 4,
            "cpuName": "synthetic",
        },
        "benchmarks": b,
    }
    handle, path = tempfile.mkstemp(suffix="-benchmarkData.json")
    with os.fdopen(handle, "w") as fh:
        json.dump(payload, fh)
    return Path(path)


def self_test() -> int:
    """Every path the gate can take, on fabricated reports.

    The numbers are synthetic and say so; what is proved is the DECISION — 9 %
    passes, 11 % and 15 % fail naming the gate, a bench that measured nothing
    fails, and another device is `incomparable` rather than red. A gate nobody
    has seen fail is a gate that does not work.
    """
    import io

    checks: list[tuple[str, bool, str]] = []

    def run(scale: dict[str, float], baseline: dict, fingerprint: str = "test/emulator/35") -> tuple[int, str]:
        path = synthetic_report(scale, fingerprint)
        try:
            out, err = io.StringIO(), io.StringIO()
            with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
                code = compare(load([path]), baseline, THRESHOLD_PCT)
            return code, out.getvalue() + err.getvalue()
        finally:
            path.unlink(missing_ok=True)

    path = synthetic_report({})
    try:
        baseline = record(load([path]), commit="self-test", threshold=THRESHOLD_PCT)
    finally:
        path.unlink(missing_ok=True)

    checks.append(("the baseline carries the four gates", set(baseline["gates"]) == {g.name for g in GATES}, ",".join(sorted(baseline["gates"]))))
    checks.append(("its p95 is the 19th of 20 samples", abs(baseline["gates"]["startup"]["p95"] - 118.0) < 1e-9, str(baseline["gates"]["startup"]["p95"])))

    code, text = run({}, baseline)
    checks.append(("the same run passes", code == 0 and "perf_gate=ok" in text, text.strip().splitlines()[-1]))

    code, text = run({g.name: 1.09 for g in GATES}, baseline)
    checks.append(("a 9 % p95 regression passes", code == 0 and "perf_gate=ok" in text, text.strip().splitlines()[-1]))

    code, text = run({g.name: 1.11 for g in GATES}, baseline)
    checks.append(
        (
            "an 11 % p95 regression fails, naming the metric and the percentage",
            code == 1 and "perf_gate=fail" in text and "+11.0 % > 10 %" in text,
            text.strip().splitlines()[-1],
        )
    )

    code, text = run({g.name: 1.15 for g in GATES}, baseline)
    checks.append(("a 15 % p95 regression fails", code == 1 and "+15.0 % > 10 %" in text, text.strip().splitlines()[-1]))

    # One gate only: the failure must name IT, not the run.
    code, text = run({"playerOpen": 1.11}, baseline)
    checks.append(
        ("one gate's regression fails the run and is named", code == 1 and "playerOpen p95" in text, text.strip().splitlines()[-1])
    )

    # A device that is not the baseline's: reported, never failed.
    code, text = run({g.name: 1.5 for g in GATES}, baseline, fingerprint="other/emulator/36")
    checks.append(("another device is incomparable, not a failure", code == 0 and "perf_gate=incomparable" in text, text.strip().splitlines()[-1]))

    # A bench that measured nothing: the gate's own message, never a silent 0.
    empty = synthetic_report({})
    data = json.loads(empty.read_text())
    for bench in data["benchmarks"]:
        if bench["name"] == "playerOpen":
            bench["metrics"] = {}
    empty.write_text(json.dumps(data))
    try:
        buf = io.StringIO()
        try:
            with contextlib.redirect_stdout(buf), contextlib.redirect_stderr(buf):
                code = compare(load([empty]), baseline, THRESHOLD_PCT)
        finally:
            empty.unlink(missing_ok=True)
        # compare() writes its "measured none" to stderr; the verdict is a fail.
        checks.append(("a bench that measured nothing fails", code == 1, buf.getvalue().strip().splitlines()[-1]))
    except SystemExit as exc:  # a missing file is not this case
        checks.append(("a bench that measured nothing fails", False, str(exc)))

    failed = [name for name, ok, _ in checks if not ok]
    for name, ok, detail in checks:
        print(f"{'PASS' if ok else 'FAIL'}  {name}  [{detail}]")
    if failed:
        print(f"self_test=FAIL ({len(failed)} of {len(checks)})")
        return 1
    print(f"self_test=PASS checks={len(checks)}")
    return 0


# --- the command ------------------------------------------------------------


def git_commit() -> str:
    try:
        return subprocess.check_output(["git", "rev-parse", "HEAD"], text=True, cwd=HERE).strip()
    except (OSError, subprocess.CalledProcessError):
        return "unknown"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Compare the Macrobenchmark reports with the nightly baseline, or record one.",
        epilog=(
            "Exit codes: 0 = every gate within the threshold (or incomparable), "
            "1 = a regression or a gate with no number, 2 = usage."
        ),
    )
    parser.add_argument(
        "reports",
        nargs="*",
        type=Path,
        help="*-benchmarkData.json files (default: every one under mobile/android/baselineprofile/build/outputs)",
    )
    parser.add_argument("--baseline", type=Path, default=HERE_BASELINE, help=f"the stored baseline (default {HERE_BASELINE})")
    parser.add_argument("--record", action="store_true", help="write the baseline from these reports")
    parser.add_argument("--commit", default=None, help="the commit the baseline is recorded at (default: git HEAD)")
    parser.add_argument("--device", default="", help="a label for the device in the baseline (e.g. 'emulator-5580')")
    parser.add_argument("--threshold", type=float, default=THRESHOLD_PCT, help=f"the p95 regression that fails, in %% (default {THRESHOLD_PCT:g})")
    parser.add_argument("--self-test", action="store_true", help="prove the gate's decision on fabricated reports")
    args = parser.parse_args(argv)

    if args.self_test:
        return self_test()

    reports = args.reports or default_reports()
    if not reports:
        parser.error("no report given and none found under mobile/android/baselineprofile/build/outputs")
    loaded = load(reports)

    if args.record:
        baseline = record(loaded, commit=args.commit or git_commit(), threshold=args.threshold, label=args.device)
        args.baseline.parent.mkdir(parents=True, exist_ok=True)
        args.baseline.write_text(json.dumps(baseline, indent=2, sort_keys=False) + "\n")
        print(f"{TOOL}: baseline written to {args.baseline} (commit {baseline['commit']}, device {baseline['device']['fingerprint']})")
        for name, got in baseline["gates"].items():
            print(f"  {name:<18} p50={fmt(got['p50'])} p95={fmt(got['p95'])} p99={fmt(got['p99'])} {got['unit']}")
        return 0

    if not args.baseline.is_file():
        print(
            f"{TOOL}: no baseline at {args.baseline}: this run has nothing to compare with. "
            "Record one with `perf-gate.py --record` on a quiet emulator run and commit it.",
            file=sys.stderr,
        )
        print("perf_gate=absent")
        return 1

    baseline = json.loads(args.baseline.read_text())
    return compare(loaded, baseline, args.threshold)


if __name__ == "__main__":
    sys.exit(main())
