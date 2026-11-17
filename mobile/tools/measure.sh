#!/usr/bin/env bash
# mobile/tools/measure.sh — the measures of docs/agents/mobile.md, one command
# each, written as JSON under ~/.cache/neezer-measure.
#
#   mobile/tools/measure.sh startup   --port 5580   # cold start, 10 iterations, profile off/on
#   mobile/tools/measure.sh frames    --port 5580   # scroll the long lists (8 000 / 4 000)
#   mobile/tools/measure.sh cpu       --port 5580 --duration 1800   # app CPU, screen off
#   mobile/tools/measure.sh memory    --port 5580   # the app's own PSS, in bytes
#   mobile/tools/measure.sh underruns  --port 5580  # AAudio: opens? clocks? xruns?
#   mobile/tools/measure.sh apk                      # APK and libnz_ffi.so sizes
#   mobile/tools/measure.sh profile   --port 5580   # generate the Baseline Profile
#
# Every number is COUNTED where a count exists (frames, bytes, ticks, xruns, PSS
# bytes); a duration is only ever the thing being measured (a start, a tap).
#
# Which emulator: the one `emu.sh start` started on --port, and no other. The
# script reads that port's PID file and refuses to run against a device it did
# not start — `adb` with no `-s` would happily talk to the person's own phone.
#
# JDK and Gradle come from env.sh: AGP 8.13 refuses JDK 25+ with an opaque
# "25.0.5", which cost MQ1 one debug cycle already.

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=mobile/tools/env.sh
. "$here/env.sh"

readonly out_dir="${NEEZER_MEASURE_STATE:-$HOME/.cache/neezer-measure}"
what="${1:-}"
[[ $# -gt 0 ]] && shift

port="${NEEZER_EMU_PORT:-5580}"
duration=""
apk_path=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --port) port="$2"; shift 2 ;;
        --port=*) port="${1#*=}"; shift ;;
        --duration) duration="$2"; shift 2 ;;
        --duration=*) duration="${1#*=}"; shift ;;
        --apk) apk_path="$2"; shift 2 ;;
        --apk=*) apk_path="${1#*=}"; shift ;;
        -h | --help | help) what="" ;;
        *) printf 'measure.sh: unknown argument: %s\n' "$1" >&2; exit 2 ;;
    esac
done

usage() {
    cat <<'EOF'
Usage: mobile/tools/measure.sh <startup|frames|cpu|memory|underruns|apk|profile> [options]

  --port <n>        the emulator emu.sh started (default $NEEZER_EMU_PORT or 5580)
  --duration <s>    cpu: seconds of screen-off playback to measure (default 300)
                    underruns: seconds the AAudio probe writes (default 5)
  --apk <path>      apk: the APK to weigh (default :app's benchmark/release build)

Writes ~/.cache/neezer-measure/<measure>-<UTC>.json, copies it to
<measure>-latest.json, prints the numbers and the path.
EOF
}

if [[ -z "$what" ]]; then
    usage
    exit 0
fi

case "$what" in
    startup | frames | cpu | memory | underruns | apk | profile) ;;
    *) printf 'measure.sh: unknown measure: %s (try --help)\n' "$what" >&2; exit 2 ;;
esac

mkdir -p "$out_dir"
stamp="$(date -u +%Y%m%dT%H%M%SZ)"
json="$out_dir/$what-$stamp.json"
log="$out_dir/$what-$stamp.log"

serial="emulator-$port"
emulator_pid_file="${NEEZER_EMU_STATE:-$HOME/.cache/neezer-emu}/$port.pid"

note() { printf 'measure.sh: %s\n' "$*" >&2; }
die() {
    printf 'measure.sh: %s\n' "$*" >&2
    exit 1
}

adb_dev() { "$(nz_adb)" -s "$serial" "$@"; }

# --- the device ---------------------------------------------------------

require_emulator() {
    [[ -f "$emulator_pid_file" ]] ||
        die "no PID file for port $port ($emulator_pid_file): start the emulator with 'emu.sh start --port $port', and never measure a device this script did not start"
    local pid
    pid="$(tr -dc '0-9' <"$emulator_pid_file")"
    [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null || die "the emulator on port $port is not running (stale PID file $emulator_pid_file)"
    [[ "$(adb_dev get-state 2>/dev/null || true)" == "device" ]] || die "$serial is not visible to adb"
    emulator_pid="$pid"
}

# What the guest's audio device IS, read from the flags emu.sh started it with:
# a measure that does not say which mode it ran in cannot be compared with the
# next one (that is the whole -no-audio vs -audio none question).
emulator_mode() {
    local cmdline
    cmdline="$(tr '\0' '\n' <"/proc/$emulator_pid/cmdline" 2>/dev/null || true)"
    if grep -qx -- '-no-audio' <<<"$cmdline"; then
        printf 'no-audio'
    elif grep -qx -- 'none' <<<"$cmdline" && grep -qx -- '-audio' <<<"$cmdline"; then
        printf 'none'
    elif grep -qx -- 'audio' <<<"$cmdline"; then
        printf 'other'
    else
        printf 'none-unknown'
    fi
}

emulator_rss_bytes() {
    awk '/VmRSS/ {print $2 * 1024}' "/proc/$emulator_pid/status" 2>/dev/null || printf '0'
}

apk_version_code() {
    local apk="$1"
    local aapt2
    aapt2="$(nz_sdk_root)/build-tools/36.0.0/aapt2"
    if [[ -x "$aapt2" && -f "$apk" ]]; then
        "$aapt2" dump badging "$apk" 2>/dev/null | sed -n "s/.*versionCode='\([0-9]*\)'.*/\1/p" | head -1
    fi
}

# One JSON header every measure carries: which device, which flags, which app.
# Built with python so a message with a quote cannot produce invalid JSON.
meta_json() {
    local mode="$1"
    printf '{"emulator_mode":"%s","emulator_pid":%s,"emulator_rss_bytes":%s,"apk_version_code":%s}' \
        "$mode" "${emulator_pid:-0}" "$(emulator_rss_bytes || printf 0)" "${2:-null}"
}

# Launches the app the way the launcher does and waits for the shell's own text,
# never a sleep: "Accueil" is a node in the accessibility tree.
APP_COMPONENT=""
resolve_launcher() {
    [[ -n "$APP_COMPONENT" ]] && return 0
    local line
    line="$(nz_launcher_component "$serial")"
    if [[ "$line" != *"org.nsupysonic.app/"* ]]; then
        # A fresh boot of the read-only emulator has no app on it: install the
        # one this worktree built, so a measure is never "not installed".
        local apk
        apk="$(find_apk)" || die "org.nsupysonic.app is not installed on $serial and no APK is built (gradle :app:assembleRelease, or pass --apk)"
        note "installing $apk on $serial"
        adb_dev install -r -g "$apk" >/dev/null 2>&1 || die "installing $apk on $serial failed"
        line="$(nz_launcher_component "$serial")"
        [[ "$line" == *"org.nsupysonic.app/"* ]] || die "org.nsupysonic.app does not resolve its launcher after installing $apk"
    fi
    APP_COMPONENT="$line"
}

launch_app() {
    resolve_launcher
    adb_dev shell am start -W -n "$APP_COMPONENT" >/dev/null 2>&1 || die "am start failed for $APP_COMPONENT"
    local started=$SECONDS
    while (( SECONDS - started < 30 )); do
        if adb_dev shell uiautomator dump /sdcard/nz-ui.xml >/dev/null 2>&1 &&
            adb_dev shell cat /sdcard/nz-ui.xml 2>/dev/null | grep -q 'text="Accueil"'; then
            return 0
        fi
    done
    die "the app's shell never showed « Accueil »"
}

app_pid() {
    adb_dev shell pidof org.nsupysonic.app 2>/dev/null | tr -d '\r' | awk '{print $1}'
}

# --- the Macrobenchmark half --------------------------------------------

# Runs one benchmark class on this emulator and turns its report into our JSON.
# The additional output is pulled from the DEVICE: Gradle's copy path has moved
# between AGP versions, the device path is the documented one.
run_bench() {
    local cls="$1"
    require_emulator
    local marker="$out_dir/.marker-$stamp-$what"
    : >"$marker"

    note "running $cls on $serial (this is the slow part: 10 cold starts)"
    # AGP's `connected…AndroidTest` runs on EVERY device adb sees: on a machine
    # where another lot's emulator is up, that would install our test APK on
    # their device and run it there (worker rule 7: never drive a device you did
    # not start). ANDROID_SERIAL is what narrows it to ours.
    export ANDROID_SERIAL="$serial"
    nz_gradle_android \
        :baselineprofile:connectedBenchmarkReleaseAndroidTest \
        -Pandroid.testInstrumentationRunnerArguments.class="$cls" 2>&1 | tee "$log" |
        grep -E "^(> Task|Starting|Tests|FAILURE|BUILD)|\bPASSED\b|\bFAILED\b" || true

    # A benchmark that did not run must not leave a green-looking JSON behind.
    grep -q "BUILD SUCCESSFUL" "$log" || {
        note "gradle failed; the last 40 lines of $log:"
        tail -n 40 "$log" >&2
        die "the benchmark did not run"
    }

    local dev_out=/sdcard/Android/media/fr.neezer.mobile.baselineprofile/additional_test_output
    local pulled="$out_dir/$what-$stamp-benchmarkData.json"
    # The device copy may live in the module's own media dir; Gradle also leaves
    # one under build/outputs. Try the host first (already pulled), then the device.
    local host_report=""
    host_report="$(find "$(nz_android_project)/baselineprofile/build/outputs" -name '*-benchmarkData.json' -newer "$marker" 2>/dev/null | sort | tail -1)"
    if [[ -z "$host_report" ]]; then
        adb_dev pull "$dev_out/." "$out_dir/$what-$stamp-device-out/" >/dev/null 2>&1 || true
        host_report="$(find "$out_dir/$what-$stamp-device-out" -name '*-benchmarkData.json' 2>/dev/null | tail -1)"
    fi
    [[ -n "$host_report" && -f "$host_report" ]] || die "no *-benchmarkData.json produced by $cls (see $log)"
    cp "$host_report" "$pulled"

    local mode vc
    mode="$(emulator_mode)"
    vc="$(apk_version_code "$(find_apk)")"
    python3 "$here/benchdata.py" --what "$what" --serial "$serial" --out "$json" \
        --meta "emulator_mode=$mode" --meta "emulator_pid=$emulator_pid" \
        --meta "emulator_rss_bytes=$(emulator_rss_bytes)" \
        --meta "apk_version_code=${vc:-null}" \
        "$pulled"
    cp "$json" "$out_dir/latest-$what.json"
    note "wrote $json"
}

find_apk() {
    if [[ -n "$apk_path" ]]; then
        printf '%s\n' "$apk_path"
        return 0
    fi
    local candidate
    for candidate in \
        "$(nz_android_project)/app/build/outputs/apk/release/app-release.apk" \
        "$(nz_android_project)/app/build/outputs/apk/benchmark/app-benchmark.apk"; do
        [[ -f "$candidate" ]] && {
            printf '%s\n' "$candidate"
            return 0
        }
    done
    return 1
}

# --- the measures -------------------------------------------------------

measure_startup() {
    run_bench fr.neezer.mobile.baselineprofile.StartupBenchmark
}

# The long-list benches (8 000 / 4 000) plus the shell's own frames: the first
# two need a scrollable list and are reported as skipped until MB8/MB9 paint one,
# the third measures the tab walk the skeleton can actually repaint.
measure_frames() {
    run_bench "fr.neezer.mobile.baselineprofile.ScrollBenchmark,fr.neezer.mobile.baselineprofile.ShellFramesBenchmark"
}

# CPU time of the APP's own process, screen off, from /proc/<pid>/stat: utime +
# stime in clock ticks (USER_HZ = 100 on Android, so a tick is 10 ms). Never the
# CPU of a shared test process, and never a percentage of the machine.
measure_cpu() {
    require_emulator
    local seconds="${duration:-300}"
    launch_app
    local pid
    pid="$(app_pid)"
    [[ -n "$pid" ]] || die "org.nsupysonic.app has no PID after launching it"

    # The screen off is what the measured case IS (playing behind a locked
    # screen); Doze is forced only when asked, because forcing it needs the
    # deviceidle command and it changes what the app may do.
    adb_dev shell input keyevent KEYCODE_SLEEP >/dev/null 2>&1 || true
    adb_dev shell dumpsys deviceidle force-idle >/dev/null 2>&1 || true

    local t0 utime0 stime0
    read -r _ utime0 stime0 <<<"$(adb_dev shell "cat /proc/$pid/stat" | awk '{print $1, $14, $15}')"
    t0=$SECONDS
    local waited=0
    while (( waited < seconds )); do
        sleep 5
        waited=$((waited + 5))
        adb_dev shell "test -d /proc/$pid" >/dev/null 2>&1 || die "the app's process died during the measure"
    done
    local elapsed=$((SECONDS - t0))
    local utime1 stime1
    read -r _ utime1 stime1 <<<"$(adb_dev shell "cat /proc/$pid/stat" | awk '{print $1, $14, $15}')"

    adb_dev shell dumpsys deviceidle unforce >/dev/null 2>&1 || true
    adb_dev shell input keyevent KEYCODE_WAKEUP >/dev/null 2>&1 || true

    local ticks=$(( (utime1 - utime0) + (stime1 - stime0) ))
    local cpu_ms=$(( ticks * 10 ))
    local mode
    mode="$(emulator_mode)"
    python3 - "$json" "$serial" "$elapsed" "$ticks" "$cpu_ms" "$pid" "$mode" <<'PY'
import json, sys
from datetime import datetime, timezone
out, serial, elapsed, ticks, cpu_ms, pid, mode = sys.argv[1:8]
elapsed, ticks, cpu_ms, pid = int(elapsed), int(ticks), int(cpu_ms), int(pid)
payload = {
    "tool": "measure.sh", "what": "cpu",
    "utc": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "serial": serial, "emulator_mode": mode,
    "screen": "off", "doze": "forced",
    "app_pid": pid, "seconds": elapsed,
    "utime_plus_stime_ticks": ticks,
    "cpu_ms": cpu_ms,
    "cpu_pct_of_one_core": round(cpu_ms / (elapsed * 1000) * 100, 3),
}
open(out, "w").write(json.dumps(payload, indent=2) + "\n")
print(f"cpu: {cpu_ms} ms over {elapsed} s of one core = {payload['cpu_pct_of_one_core']} % (pid {pid}, screen off, doze forced)")
print(f"json: {out}")
PY
    cp "$json" "$out_dir/latest-cpu.json"
}

# The APP's own PSS, in bytes. `dumpsys meminfo <package>` reports only that
# process (the rule: never the RSS of a shared test process). PSS is the honest
# per-process number — proportional set size — and this script multiplies it to
# BYTES because the plan's ceilings are in bytes.
measure_memory() {
    require_emulator
    launch_app
    local mode out
    mode="$(emulator_mode)"
    local raw="$out_dir/memory-$stamp-meminfo.txt"
    adb_dev shell dumpsys meminfo org.nsupysonic.app >"$raw" 2>&1 || die "dumpsys meminfo failed"
    python3 - "$json" "$serial" "$mode" "$raw" <<'PY'
import json, re, sys
from datetime import datetime, timezone
out, serial, mode, raw = sys.argv[1:5]
text = open(raw, encoding="utf-8", errors="replace").read()
pss_kb = None
m = re.search(r"TOTAL PSS:\s+([0-9]+)", text)
if m:
    pss_kb = int(m.group(1))
else:
    # Older dumpsys: the TOTAL row's first column.
    m = re.search(r"^\s*TOTAL\s+([0-9]+)", text, re.M)
    if m:
        pss_kb = int(m.group(1))
if pss_kb is None:
    print("measure.sh: could not read TOTAL PSS from dumpsys meminfo", file=sys.stderr)
    sys.exit(1)
payload = {
    "tool": "measure.sh", "what": "memory",
    "utc": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "serial": serial, "emulator_mode": mode,
    "process": "org.nsupysonic.app",
    "pss_bytes": pss_kb * 1024,
    "pss_kb": pss_kb,
    "source": raw,
}
open(out, "w").write(json.dumps(payload, indent=2) + "\n")
print(f"memory: PSS {pss_kb * 1024} bytes ({pss_kb} kB) for org.nsupysonic.app")
print(f"json: {out}")
PY
    cp "$json" "$out_dir/latest-memory.json"
}

# The AAudio question the note asks MQ1 to answer: does the guest's audio device
# OPEN and CLOCK under the emulator's silent flags? Compiled with the NDK for the
# emulator's ABI, pushed to the guest's /data/local/tmp (not the host's /tmp, and
# not the repository), run, removed.
measure_underruns() {
    require_emulator
    local seconds="${duration:-5}"
    local ndk clang abi probe_bin
    ndk="$(find "$(nz_sdk_root)/ndk" -maxdepth 1 -mindepth 1 -type d | sort | tail -1)"
    [[ -n "$ndk" ]] || die "no NDK under $(nz_sdk_root)/ndk (needed to build the AAudio probe)"
    abi="$(adb_dev shell getprop ro.product.cpu.abi | tr -d '\r')"
    case "$abi" in
        x86_64) clang="$ndk/toolchains/llvm/prebuilt/linux-x86_64/bin/x86_64-linux-android28-clang"; android_abi=x86_64 ;;
        arm64-v8a) clang="$ndk/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android28-clang"; android_abi=arm64-v8a ;;
        *) die "unsupported ABI $abi for the AAudio probe" ;;
    esac
    [[ -x "$clang" ]] || die "no NDK clang at $clang"
    mkdir -p "$out_dir/aaudio"
    probe_bin="$out_dir/aaudio/aaudio-probe-$android_abi"
    "$clang" -O2 -Wall -Wextra -o "$probe_bin" "$here/aaudio-probe.c" -laaudio ||
        die "the AAudio probe did not compile"

    adb_dev push "$probe_bin" /data/local/tmp/nz-aaudio-probe >/dev/null
    local mode
    mode="$(emulator_mode)"
    local raw="$out_dir/underruns-$stamp-probe.json"
    adb_dev shell "/data/local/tmp/nz-aaudio-probe $seconds $mode" >"$raw" 2>"$out_dir/underruns-$stamp-probe.err" || true
    adb_dev shell rm -f /data/local/tmp/nz-aaudio-probe >/dev/null 2>&1 || true

    python3 - "$json" "$serial" "$mode" "$seconds" "$raw" "$(nz_sdk_root)" <<'PY'
import json, sys
from datetime import datetime, timezone
out, serial, mode, seconds, raw, sdk = sys.argv[1:7]
line = [l for l in open(raw, encoding="utf-8", errors="replace").read().splitlines() if l.strip().startswith("{")]
if not line:
    print(f"measure.sh: the AAudio probe printed no JSON (see {raw})", file=sys.stderr)
    sys.exit(1)
probe = json.loads(line[-1])
payload = {
    "tool": "measure.sh", "what": "underruns",
    "utc": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "serial": serial,
    "asked_seconds": int(seconds),
    "probe": probe,
    "source": raw,
}
open(out, "w").write(json.dumps(payload, indent=2) + "\n")
if not probe.get("opened"):
    print(f"underruns: AAudio did NOT open in mode {mode} (code {probe.get('code')}, {probe.get('error')})")
else:
    print(
        "underruns: AAudio opened in mode {m}: rate {rate} Hz, burst {burst_frames} frames, "
        "buffer {buffer_frames} frames, callbacks {callbacks}, frames {frames}, "
        "ticks {ticks:.3f} (steady {ticks_steady:.3f}), xruns {xruns}".format(
            m=mode,
            rate=probe.get("rate"),
            burst_frames=probe.get("burst_frames"),
            buffer_frames=probe.get("buffer_frames"),
            callbacks=probe.get("callbacks"),
            frames=probe.get("frames"),
            ticks=float(probe.get("ticks") or 0.0),
            ticks_steady=float(probe.get("ticks_steady") or 0.0),
            xruns=probe.get("xruns"),
        )
    )
print(f"json: {out}")
PY
    cp "$json" "$out_dir/latest-underruns.json"
}

# Sizes in BYTES: the APK, its dex, and each ABI's libnz_ffi.so — the plan's
# ceilings are universal ≤ 45 MB, arm64 ≤ 18 MB, the .so ≤ 9 MB.
measure_apk() {
    local apk
    apk="$(find_apk)" || die "no APK: build one (gradle :app:assembleRelease) or pass --apk"
    [[ -f "$apk" ]] || die "no APK at $apk"
    python3 - "$json" "$apk" <<'PY'
import json, subprocess, sys, zipfile
from datetime import datetime, timezone
out, apk = sys.argv[1:3]
z = zipfile.ZipFile(apk)
entries = {i.filename: i.file_size for i in z.infolist()}
libs = {n: s for n, s in entries.items() if n.startswith("lib/") and n.endswith(".so")}
dex = sum(s for n, s in entries.items() if n.endswith(".dex"))
payload = {
    "tool": "measure.sh", "what": "apk",
    "utc": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "apk": apk,
    "apk_bytes": subprocess.check_output(["stat", "-c", "%s", apk]).decode().strip(),
    "dex_bytes": dex,
    "libs": {n: s for n, s in sorted(libs.items())},
    "entries": len(entries),
}
payload["apk_bytes"] = int(payload["apk_bytes"])
open(out, "w").write(json.dumps(payload, indent=2) + "\n")
print(f"apk: {payload['apk_bytes']} bytes ({apk})")
for n, s in sorted(libs.items()):
    print(f"  {n}: {s} bytes")
print(f"json: {out}")
PY
    cp "$json" "$out_dir/latest-apk.json"
}

# Generates the Baseline Profile: runs BaselineProfileGenerator, then reports what
# actually happened — a `.txt`, or the library's own reason for skipping.
#
# `BaselineProfileRule` REFUSES an emulator by itself ("Skipping test because
# it's running on emulator and `skipOnEmulator` is enabled"), and neither
# `androidx.benchmark.suppressErrors` nor
# `androidx.benchmark.skipBenchmarksOnEmulator=false` changes that: the guard is
# inside the profile generator, not the error checker. So on an emulator this
# measure reports `status=skipped` with that cause, and fails only when the test
# FAILS — a skip is the tool's own decision, not ours, and the profile the release
# ships is MQ2's, generated on a phone over the whole journey.
#
# The `.txt` lands under `:app/src/<variant>/generated/baselineProfiles/`; it is
# generated output and is never committed by this lot (MQ2 owns the shipped one).
measure_profile() {
    require_emulator
    local marker="$out_dir/.marker-$stamp-profile"
    : >"$marker"
    note "running BaselineProfileGenerator on $serial"
    export ANDROID_SERIAL="$serial"
    nz_gradle_android \
        :baselineprofile:connectedBenchmarkReleaseAndroidTest \
        -Pandroid.testInstrumentationRunnerArguments.class=fr.neezer.mobile.baselineprofile.BaselineProfileGenerator \
        2>&1 | tee "$log" | grep -E "^(> Task :baselineprofile:connected|Starting|Finished)|SKIPPED|FAILED|BUILD " || true

    # The report's file name contains SPACES ("TEST-emulator-5580 - 15-….xml"), so
    # a newline-joined list passed through `$(…)` and split on whitespace would
    # destroy it: python gets the directory and the marker and globs itself.
    local txt
    txt="$(find "$(nz_android_project)/app/src" -name 'baseline-prof.txt' -newer "$marker" 2>/dev/null | tail -1)"

    python3 - "$json" "$serial" \
        "$(nz_android_project)/baselineprofile/build/outputs/androidTest-results/connected" \
        "$marker" "$txt" "$log" <<'PYEOF'
import glob, json, os, re, sys
from datetime import datetime, timezone

out, serial, results_dir, marker, txt, log = sys.argv[1:7]
marker_mtime = os.path.getmtime(marker) if os.path.exists(marker) else 0.0
tests = []
for path in glob.glob(os.path.join(results_dir, "**", "*.xml"), recursive=True):
    if os.path.getmtime(path) < marker_mtime:
        continue
    text = open(path, encoding="utf-8", errors="replace").read()
    for m in re.finditer(r'<testcase name="([^"]+)"[^>]*>(.*?)</testcase>', text, re.S):
            body = m.group(2)
            if "<skipped" in body:
                skipped = re.search(r"<skipped[^>]*/?>(.*?)(</skipped>|</testcase>)", body, re.S)
                tests.append({"name": m.group(1), "status": "skipped", "cause": (skipped.group(1).strip() if skipped else "")})
            elif "<failure" in body:
                failure = re.search(r"<failure[^>]*>(.*?)</failure>", body, re.S)
                tests.append({"name": m.group(1), "status": "failed", "cause": failure.group(1).strip()[:400]})
            else:
                tests.append({"name": m.group(1), "status": "passed", "cause": ""})

if txt:
    status = "generated"
elif any(t["status"] == "failed" for t in tests):
    status = "failed"
elif any(t["status"] == "skipped" for t in tests):
    status = "skipped"
else:
    status = "unknown"

cause = ""
if status == "skipped":
    cause = next((t["cause"] for t in tests if t["status"] == "skipped" and t["cause"]), "")
    if not cause:
        cause = "BaselineProfileRule refuses an emulator (its own skipOnEmulator guard); see " + log
elif status == "failed":
    cause = next((t["cause"] for t in tests if t["status"] == "failed"), "")

payload = {
    "tool": "measure.sh",
    "what": "profile",
    "utc": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "serial": serial,
    "status": status,
    "cause": cause,
    "profile_path": txt or None,
    "profile_bytes": (len(open(txt, "rb").read()) if txt else 0),
    "tests": tests,
    "log": log,
}
open(out, "w").write(json.dumps(payload, indent=2) + "\n")

if status == "generated":
    print(f"profile: {txt} ({payload['profile_bytes']} bytes) — generated, NOT committed (MQ2 owns the shipped profile)")
elif status == "failed":
    print(f"profile: the generator FAILED — {cause}")
    sys.exit(1)
else:
    print(f"profile: status={status} — {cause}")
print(f"json: {out}")
PYEOF
    cp "$json" "$out_dir/latest-profile.json"
}

case "$what" in
    startup) measure_startup ;;
    frames) measure_frames ;;
    cpu) measure_cpu ;;
    memory) measure_memory ;;
    underruns) measure_underruns ;;
    apk) measure_apk ;;
    profile) measure_profile ;;
esac
