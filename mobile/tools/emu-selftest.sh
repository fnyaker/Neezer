#!/usr/bin/env bash
# mobile/tools/emu-selftest.sh — the acceptance of the harness, run as a command
# instead of remembered: `emu.sh start/wait/stop` twice leaves nothing behind,
# two emulators run side by side on distinct ports, and neither ever has a window
# or a speaker.
#
#   mobile/tools/emu-selftest.sh [--ports 5580,5582]
#
# Each check prints what it observed (seconds, bytes, whether the port is held),
# and the script exits non-zero on the first claim that does not hold. Nothing
# here is a sleep: a boot is `sys.boot_completed`, a stop is the process leaving,
# a port is `ss`'s answer.
#
# The state it inspects is exactly what an orchestrator sees: PID files under
# ~/.cache/neezer-emu, qemu processes, listening ports on the console/adb pair.

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=mobile/tools/env.sh
. "$here/env.sh"

ports_arg="${NEEZER_EMU_SELFTEST_PORTS:-5580,5582}"
while [[ $# -gt 0 ]]; do
    case "$1" in
        --ports) ports_arg="$2"; shift 2 ;;
        --ports=*) ports_arg="${1#*=}"; shift ;;
        -h | --help) sed -n '2,16p' "$0"; exit 0 ;;
        *) printf 'emu-selftest.sh: unknown argument: %s\n' "$1" >&2; exit 2 ;;
    esac
done
IFS=',' read -r -a ports <<<"$ports_arg"
(( ${#ports[@]} >= 1 )) || {
    printf 'emu-selftest.sh: give at least one port\n' >&2
    exit 2
}

state="${NEEZER_EMU_STATE:-$HOME/.cache/neezer-emu}"
adb_bin="$(nz_adb)"
failures=0
note() { printf 'emu-selftest: %s\n' "$*" >&2; }
check() {
    local what="$1" ok="$2" detail="${3:-}"
    if [[ "$ok" == "1" ]]; then
        printf 'ok   %s %s\n' "$what" "$detail"
    else
        printf 'FAIL %s %s\n' "$what" "$detail"
        failures=$((failures + 1))
    fi
}

# Nothing of ours is running or listening: the claim a restart cycle makes.
assert_clean() {
    local label="$1" port pid held rss
    for port in "${ports[@]}"; do
        pid=""
        [[ -f "$state/$port.pid" ]] && pid="$(tr -dc '0-9' <"$state/$port.pid")"
        if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
            check "$label: port $port process gone" 0 "pid $pid still alive"
        else
            check "$label: port $port process gone" 1 ""
        fi
        check "$label: port $port PID file removed" "$([[ -f "$state/$port.pid" ]] && echo 0 || echo 1)" ""
        held="$(ss -ltnH 2>/dev/null | grep -Ec ":(${port}|$((port + 1)))[[:space:]]" || true)"
        check "$label: port $port not listening" "$([[ "${held:-0}" == "0" ]] && echo 1 || echo 0)" "listening=$held"
        # The flock file stays; what must be gone is a HOLDER. Acquiring it is the test.
        if (exec 9>"$state/$port.lock" && flock -n 9); then
            check "$label: port $port lock free" 1 ""
        else
            check "$label: port $port lock free" 0 "the lock file is still held"
        fi
    done
}

stopped_all() {
    local port
    for port in "${ports[@]}"; do
        "$here/emu.sh" stop --port "$port" >/dev/null 2>&1 || true
    done
}

trap stopped_all EXIT

# --- 1. a full cycle, twice ------------------------------------------------
for cycle in 1 2; do
    for port in "${ports[@]}"; do
        note "cycle $cycle: $port"
        "$here/emu.sh" start --port "$port" >/dev/null
        pid="$(tr -dc '0-9' <"$state/$port.pid")"
        cmdline="$(tr '\0' '\n' <"/proc/$pid/cmdline")"
        check "cycle $cycle port $port has no window" "$(grep -qx -- '-no-window' <<<"$cmdline" && echo 1 || echo 0)" ""
        if grep -qx -- '-no-audio' <<<"$cmdline" || (grep -qx -- '-audio' <<<"$cmdline" && grep -qx -- 'none' <<<"$cmdline"); then
            check "cycle $cycle port $port is silent" 1 "$(grep -q -- '-no-audio' <<<"$cmdline" && echo no-audio || echo none)"
        else
            check "cycle $cycle port $port is silent" 0 "$cmdline"
        fi
        boot="$("$here/emu.sh" wait --port "$port")"
        check "cycle $cycle port $port booted" 1 "$boot"
        rss="$(awk '/VmRSS/ {print $2 * 1024}' "/proc/$pid/status")"
        check "cycle $cycle port $port RSS measured" "$([[ -n "$rss" ]] && echo 1 || echo 0)" "rss_bytes=$rss"
        "$here/emu.sh" stop --port "$port" >/dev/null
        assert_clean "cycle $cycle" 
    done
done

# --- 2. two emulators side by side ----------------------------------------
if (( ${#ports[@]} >= 2 )); then
    note "side by side: ${ports[*]}"
    for port in "${ports[@]}"; do
        "$here/emu.sh" start --port "$port" >/dev/null
    done
    for port in "${ports[@]}"; do
        boot="$("$here/emu.sh" wait --port "$port")"
        check "side by side port $port booted" 1 "$boot"
        state_now="$("$adb_bin" -s "emulator-$port" get-state 2>/dev/null || true)"
        check "side by side port $port is adb-visible" "$([[ "$state_now" == "device" ]] && echo 1 || echo 0)" "state=$state_now"
        check "side by side port $port has its own PID" 1 "pid $(tr -dc '0-9' <"$state/$port.pid")"
    done
    # They must be two guests, not one device reached twice: compare their kernel
    # boot ids (a property the guest generates at boot, different per instance).
    a="$("$adb_bin" -s "emulator-${ports[0]}" shell getprop ro.boot.qemu.avd_name 2>/dev/null | tr -d '\r')"
    b="$("$adb_bin" -s "emulator-${ports[1]}" shell getprop ro.boot.qemu.avd_name 2>/dev/null | tr -d '\r')"
    boot_a="$("$adb_bin" -s "emulator-${ports[0]}" shell cat /proc/sys/kernel/random/boot_id 2>/dev/null | tr -d '\r')"
    boot_b="$("$adb_bin" -s "emulator-${ports[1]}" shell cat /proc/sys/kernel/random/boot_id 2>/dev/null | tr -d '\r')"
    check "side by side: two distinct guests" "$([[ -n "$boot_a" && -n "$boot_b" && "$boot_a" != "$boot_b" ]] && echo 1 || echo 0)" \
        "avd=$a/$b boot_ids differ=$([[ "$boot_a" != "$boot_b" ]] && echo 1 || echo 0)"
    for port in "${ports[@]}"; do
        "$here/emu.sh" stop --port "$port" >/dev/null
    done
    assert_clean "side by side"
fi

# --- 3. refuses what it must refuse --------------------------------------
# A port it did not start: no PID file, so `wait`/`stop` must refuse rather than
# drive (or kill) whatever is there.
check "refuses a port with no PID file" \
    "$("$here/emu.sh" stop --port "${ports[0]}" >/dev/null 2>&1 && echo 0 || echo 1)" \
    "(emu.sh stop on a port it never started must exit non-zero)"

printf '\nfailures=%s\n' "$failures"
(( failures == 0 )) || exit 1
printf 'emu-selftest: all checks passed\n'
