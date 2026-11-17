#!/usr/bin/env bash
# mobile/tools/emu.sh — the ONLY way a lot starts an Android emulator.
#
#   mobile/tools/emu.sh create                # the AVD, once per machine
#   mobile/tools/emu.sh start  --port 5580    # headless, KVM, no window, no sound
#   mobile/tools/emu.sh wait   --port 5580    # until sys.boot_completed=1 (an event)
#   mobile/tools/emu.sh stop   --port 5580    # adb emu kill, then OUR pid only
#   mobile/tools/emu.sh status --port 5580
#
# Why every rule lives here, once (docs/agents/mobile.md, "The emulator,
# headless, as the user"): an emulator is a qemu process holding ~3.8 GB, it
# plays its sound on the HOST and opens a window on the USER's display unless
# told otherwise, and a stray `adb` command picks whatever phone is plugged in.
# A lot that copies the flags by hand gets one of those three wrong eventually,
# so the flags, the PID bookkeeping, the memory guard and the port rules are
# written once and every lot calls this script.
#
# The absolute rules this file enforces:
#   - `-no-window` and a silent audio path, always (nothing on the display,
#     nothing in the speakers);
#   - every adb call is `adb -s emulator-<port>` (a bare adb may pick the
#     person's own phone, and then `install` overwrites their app);
#   - only a PID this script wrote is ever signalled (never pkill/killall);
#   - at most two emulators at a time on this machine;
#   - a port is only ever touched when this script started it.
#
# State (PID files, logs) lives under ~/.cache/neezer-emu, never /tmp: /tmp is
# a tmpfs here, so a multi-gigabyte emulator user-data there is RAM.

set -euo pipefail

readonly AVD="${NEEZER_AVD:-nz35}"
readonly IMAGE="system-images;android-35;default;x86_64"
readonly DEVICE_PROFILE="pixel_6"
readonly MAX_EMULATORS=2
readonly MIN_MEM_KB="${NEEZER_EMU_MIN_MEM_KB:-8388608}" # 8 GB, measured headroom

# Where the SDK, the AVD and the per-port state live. The order matters: a CI
# runner has ANDROID_SDK_ROOT, this machine has ANDROID_HOME, and the AVD home
# is the one MB0's prototype created, so a lot never re-creates an AVD.
sdk_root() {
    if [[ -n "${ANDROID_SDK_ROOT:-}" ]]; then
        printf '%s\n' "$ANDROID_SDK_ROOT"
    elif [[ -n "${ANDROID_HOME:-}" ]]; then
        printf '%s\n' "$ANDROID_HOME"
    else
        printf '%s\n' "$HOME/.local/share/android-sdk"
    fi
}

avd_home() {
    if [[ -n "${ANDROID_AVD_HOME:-}" ]]; then
        printf '%s\n' "$ANDROID_AVD_HOME"
    elif [[ -d "$HOME/.local/share/android-avd/$AVD.avd" ]]; then
        printf '%s\n' "$HOME/.local/share/android-avd"
    elif [[ -n "${NEEZER_AVD_HOME:-}" ]]; then
        printf '%s\n' "$NEEZER_AVD_HOME"
    else
        # avdmanager's own default; a CI job creates its AVD there.
        printf '%s\n' "$HOME/.android/avd"
    fi
}

state_dir() { printf '%s\n' "${NEEZER_EMU_STATE:-$HOME/.cache/neezer-emu}"; }
adb_bin() { printf '%s\n' "$(sdk_root)/platform-tools/adb"; }
emulator_bin() { printf '%s\n' "$(sdk_root)/emulator/emulator"; }
# Always through this wrapper: it is the absolute path of THIS sdk's adb, and
# every caller passes -s emulator-<port> (a bare adb picks whatever phone is
# plugged in).
adb() { "$(adb_bin)" "$@"; }
serial_of() { printf 'emulator-%s\n' "$1"; }
pid_file_of() { printf '%s/%s.pid\n' "$(state_dir)" "$1"; }
log_file_of() { printf '%s/%s.log\n' "$(state_dir)" "$1"; }

die() {
    printf 'emu.sh: %s\n' "$*" >&2
    exit 1
}

note() { printf 'emu.sh: %s\n' "$*" >&2; }

usage() {
    cat <<'EOF'
Usage: mobile/tools/emu.sh <command> --port <n> [options]

Commands:
  create                Create the AVD (nz35, API 35 x86_64) if it is missing.
  start  --port <n>     Start the emulator headless on ports n (console) and n+1 (adb).
  wait   --port <n>     Block until sys.boot_completed=1. An event, never a sleep.
  stop   --port <n>     adb emu kill, then SIGTERM the PID this script wrote.
  status --port <n>     Print port, pid, liveness and boot state.

Options:
  --port <n>            Even port, 5554-5682 (the emulator's own rule).
  --audio <mode>        no-audio (default) or none. Both are silent on the host;
                        see docs/agents/mobile.md, "Measures" (MQ1 measured which
                        one still gives the guest a working audio clock).
  --timeout <seconds>   wait's boot budget (default 300, or NEEZER_EMU_BOOT_TIMEOUT).

Environment (all optional, read once):
  ANDROID_HOME / ANDROID_SDK_ROOT   the SDK (default ~/.local/share/android-sdk)
  ANDROID_AVD_HOME                  the AVD directory (default ~/.local/share/android-avd
                                    when the AVD is there, else ~/.android/avd)
  NEEZER_EMU_STATE                  PID files and logs (default ~/.cache/neezer-emu)
  NEEZER_EMU_MIN_MEM_KB             start refuses below this MemAvailable (8 GB)

Every adb call is `adb -s emulator-<port>`: this script never talks to a device
it did not start, and never kills a PID it did not write.
EOF
}

# --- arguments --------------------------------------------------------------

command="${1:-}"
[[ $# -gt 0 ]] && shift
port=""
audio="no-audio"
boot_timeout="${NEEZER_EMU_BOOT_TIMEOUT:-300}"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --port)
            [[ $# -ge 2 ]] || die "--port needs a value"
            port="$2"
            shift 2
            ;;
        --port=*)
            port="${1#*=}"
            shift
            ;;
        --audio)
            [[ $# -ge 2 ]] || die "--audio needs a value"
            audio="$2"
            shift 2
            ;;
        --audio=*)
            audio="${1#*=}"
            shift
            ;;
        --timeout)
            [[ $# -ge 2 ]] || die "--timeout needs a value"
            boot_timeout="$2"
            shift 2
            ;;
        -h | --help)
            usage
            exit 0
            ;;
        *) die "unknown argument: $1 (try --help)" ;;
    esac
done

case "$command" in
    create | start | wait | stop | status) ;;
    "" | -h | --help | help)
        usage
        exit 0
        ;;
    *) die "unknown command: $command (try --help)" ;;
esac

if [[ "$command" == "create" ]]; then
    port="${port:-5554}" # an AVD is not per-port; a placeholder for the checks below
fi

# An even port in 5554-5682: the emulator pairs console=PORT and adb=PORT+1 and
# refuses an odd one. A port per worker is what lets two lots coexist.
[[ "$port" =~ ^[0-9]+$ ]] || die "--port <n> is required and must be a number"
(( port >= 5554 && port <= 5682 )) || die "port $port is outside the emulator's range 5554-5682"
(( port % 2 == 0 )) || die "port $port must be even (the emulator uses <port> console, <port+1> adb)"

case "$audio" in
    no-audio) audio_flag="-no-audio" ;;
    none) audio_flag="-audio none" ;;
    *) die "--audio must be no-audio or none (got $audio)" ;;
esac

readonly serial="$(serial_of "$port")"
readonly pid_file="$(pid_file_of "$port")"
readonly log_file="$(log_file_of "$port")"

# --- helpers ----------------------------------------------------------------

# The PID this script wrote for <port>, or nothing. Never guesses.
own_pid() {
    [[ -f "$pid_file" ]] || return 1
    local pid
    pid="$(tr -dc '0-9' <"$pid_file")"
    [[ -n "$pid" ]] || return 1
    printf '%s\n' "$pid"
}

# Is <pid> alive AND the emulator this script started for <port>? A PID can be
# reused by anything after a reboot; its command line is what proves it is ours.
pid_is_our_emulator() {
    local pid="$1" cmdline
    [[ -r "/proc/$pid/cmdline" ]] || return 1
    cmdline="$(tr '\0' ' ' <"/proc/$pid/cmdline")"
    [[ "$cmdline" == *"emulator"* ]] || return 1
    [[ "$cmdline" == *"-port $port"* || "$cmdline" == *"-port=$port"* ]] || return 1
    return 0
}

require_ours() {
    local pid
    pid="$(own_pid)" || die "no PID file for port $port ($pid_file): this script did not start it, and it never touches a port it did not start"
    pid_is_our_emulator "$pid" || die "the PID file for port $port names $pid, which is not this script's emulator: refusing to touch it (check the process by hand)"
    printf '%s\n' "$pid"
}

# A listening socket on the console or the adb port that we did not start is
# somebody else's emulator (`adb -s emulator-<port>` would then drive THEIR
# device, and their window/sound are not ours to stop).
port_held_by_other() {
    local la
    la="$(ss -ltnH 2>/dev/null || true)"
    grep -Eq ":(${port}|$((port + 1)))[[:space:]]" <<<"$la"
}

live_emulators() {
    local f pid n=0
    for f in "$(state_dir)"/*.pid; do
        [[ -f "$f" ]] || continue
        pid="$(tr -dc '0-9' <"$f")"
        [[ -n "$pid" ]] || continue
        [[ -r "/proc/$pid/cmdline" ]] || continue
        tr '\0' ' ' <"/proc/$pid/cmdline" | grep -q "emulator" || continue
        n=$((n + 1))
    done
    printf '%s\n' "$n"
}

require_sdk() {
    [[ -x "$(adb_bin)" ]] || die "adb not found at $(adb_bin): set ANDROID_HOME / ANDROID_SDK_ROOT"
    [[ -x "$(emulator_bin)" ]] || die "emulator not found at $(emulator_bin): set ANDROID_HOME / ANDROID_SDK_ROOT"
}

require_kvm() {
    [[ -r /dev/kvm && -w /dev/kvm ]] ||
        die "/dev/kvm is not readable+writable by $(id -un): KVM acceleration needs it (this machine: crw-rw-rw-)"
}

# --- commands ---------------------------------------------------------------

cmd_create() {
    require_sdk
    local avd_dir home
    home="$(avd_home)"
    avd_dir="$home/$AVD.avd"
    if [[ -d "$avd_dir" ]]; then
        note "AVD $AVD already exists ($avd_dir)"
        return 0
    fi
    local sdkmanager avdmanager
    sdkmanager="$(sdk_root)/cmdline-tools/latest/bin/sdkmanager"
    avdmanager="$(sdk_root)/cmdline-tools/latest/bin/avdmanager"
    [[ -x "$sdkmanager" ]] || die "sdkmanager not found at $sdkmanager"
    [[ -x "$avdmanager" ]] || die "avdmanager not found at $avdmanager"

    if [[ ! -d "$(sdk_root)/system-images/android-35/default/x86_64" ]]; then
        note "installing $IMAGE, platform-tools and the emulator (a few GB, on disk)"
        yes | "$sdkmanager" --licenses >/dev/null
        "$sdkmanager" "platform-tools" "emulator" "$IMAGE"
    fi

    note "creating AVD $AVD ($IMAGE, $DEVICE_PROFILE)"
    mkdir -p "$home"
    echo no | ANDROID_AVD_HOME="$home" "$avdmanager" create avd \
        -n "$AVD" -k "$IMAGE" --device "$DEVICE_PROFILE" --force
    [[ -d "$avd_dir" ]] || die "avdmanager did not create $avd_dir"
    note "AVD $AVD created ($avd_dir)"
}

cmd_start() {
    require_sdk
    require_kvm
    mkdir -p "$(state_dir)"

    # One start per port at a time, so two lots racing on the same port get one
    # winner and one clear message instead of two qemu both dying on a lock.
    exec 9>"$(state_dir)/$port.lock"
    flock -n 9 || die "another emu.sh is starting port $port right now"

    if pid="$(own_pid)" && pid_is_our_emulator "$pid"; then
        die "port $port already runs (pid $pid, serial $serial); stop it first or pick another port"
    fi
    rm -f "$pid_file"

    if port_held_by_other; then
        die "port $port (or $((port + 1))) is already listening and this script did not start it: refusing to drive somebody else's emulator"
    fi

    local count
    count="$(live_emulators)"
    if (( count >= MAX_EMULATORS )); then
        die "$count emulators are already running (at most $MAX_EMULATORS on this machine, ~3.8 GB each); stop one first"
    fi

    local mem_kb
    mem_kb="$(awk '/^MemAvailable:/ {print $2}' /proc/meminfo)"
    if (( mem_kb < MIN_MEM_KB )); then
        die "MemAvailable is $((mem_kb / 1024)) MB, below the $((MIN_MEM_KB / 1024)) MB guard: an emulator would swap the whole machine"
    fi

    [[ -d "$(avd_home)/$AVD.avd" ]] || die "no AVD $AVD under $(avd_home): run 'emu.sh create' first"

    # The flags are the contract (docs/agents/mobile.md):
    #  -no-window            nothing on the person's display
    #  -no-audio|-audio none nothing on the person's speakers
    #  -gpu swiftshader_indirect  software GL: no GPU contention with the desktop
    #  -no-boot-anim         a deterministic boot time (it is measured)
    #  -no-snapshot          always a COLD boot, so a measure is not a warm one
    #  -read-only            several emulators can share one AVD
    #  -memory 3072 -cores 4 the measured-good profile (MQ1)
    ANDROID_AVD_HOME="$(avd_home)" \
        setsid "$(emulator_bin)" \
        -avd "$AVD" \
        -port "$port" \
        -no-window \
        $audio_flag \
        -gpu swiftshader_indirect \
        -no-boot-anim \
        -no-snapshot \
        -read-only \
        -no-metrics \
        -memory 3072 \
        -cores 4 \
        9>&- \
        >"$log_file" 2>&1 &
    local pid=$!

    printf '%s\n' "$pid" >"$pid_file"

    # The process must survive its first second: an emulator that cannot open
    # KVM (or whose AVD is locked) exits immediately, and a PID file pointing at
    # a dead process is worse than none.
    local waited=0
    while (( waited < 50 )); do
        if ! kill -0 "$pid" 2>/dev/null; then
            rm -f "$pid_file"
            note "the emulator exited at once; last log lines ($log_file):"
            tail -n 20 "$log_file" >&2 || true
            die "start failed on port $port"
        fi
        if grep -q "Successfully loaded snapshot\|boot completed\|emulator: INFO: boot completed" "$log_file" 2>/dev/null; then
            break
        fi
        sleep 0.1
        waited=$((waited + 1))
    done

    note "started $serial (pid $pid, $audio, gpu swiftshader_indirect, 3072 MB, 4 cores)"
    note "log: $log_file"
    printf 'port=%s pid=%s serial=%s\n' "$port" "$pid" "$serial"
}

cmd_wait() {
    require_sdk
    local pid
    pid="$(require_ours)"

    # Starting the adb server names no device, so it cannot touch the person's
    # phone; every device command below is -s emulator-<port>.
    adb start-server >/dev/null 2>&1 || true

    local started=$SECONDS
    while :; do
        if ! kill -0 "$pid" 2>/dev/null; then
            note "the emulator died while booting; last log lines ($log_file):"
            tail -n 20 "$log_file" >&2 || true
            die "boot failed on port $port"
        fi

        # sys.boot_completed is the event; a sleep-and-hope is exactly what the
        # note forbids. `adb wait-for-device` first, so the first getprop is not
        # racing the transport coming up.
        if [[ "$(adb -s "$serial" get-state 2>/dev/null || true)" == "device" ]]; then
            local done_flag
            done_flag="$(adb -s "$serial" shell getprop sys.boot_completed 2>/dev/null | tr -d '\r\n' || true)"
            if [[ "$done_flag" == "1" ]]; then
                local secs=$((SECONDS - started))
                note "$serial booted in ${secs}s"
                printf 'port=%s booted=1 seconds=%s\n' "$port" "$secs"
                return 0
            fi
        fi

        if (( SECONDS - started >= boot_timeout )); then
            die "$serial did not reach sys.boot_completed=1 within ${boot_timeout}s"
        fi
        sleep 0.2
    done
}

cmd_stop() {
    require_sdk
    local pid
    pid="$(require_ours)"

    # The documented stop: the guest shuts itself down, so no image is left
    # half-written. The PID is the fallback and it is OURS.
    adb -s "$serial" emu kill >/dev/null 2>&1 || true

    local waited=0
    while (( waited < 150 )) && kill -0 "$pid" 2>/dev/null; do
        sleep 0.2
        waited=$((waited + 1))
    done
    if kill -0 "$pid" 2>/dev/null; then
        note "adb emu kill did not stop pid $pid in 30 s; sending SIGTERM to it only"
        kill -TERM "$pid" 2>/dev/null || true
        waited=0
        while (( waited < 50 )) && kill -0 "$pid" 2>/dev/null; do
            sleep 0.2
            waited=$((waited + 1))
        done
    fi
    kill -0 "$pid" 2>/dev/null && die "pid $pid is still alive; it is this script's emulator, so stop it by hand (kill -TERM $pid) — never pkill"

    rm -f "$pid_file"
    note "stopped $serial (pid $pid)"
    printf 'port=%s stopped=1\n' "$port"
}

cmd_status() {
    local pid alive=0 booted=""
    if pid="$(own_pid)"; then
        if kill -0 "$pid" 2>/dev/null; then alive=1; fi
        if (( alive == 1 )) && pid_is_our_emulator "$pid" 2>/dev/null; then
            if [[ -x "$(adb_bin)" ]] && [[ "$(adb -s "$serial" get-state 2>/dev/null || true)" == "device" ]]; then
                booted="$(adb -s "$serial" shell getprop sys.boot_completed 2>/dev/null | tr -d '\r\n' || true)"
            fi
        fi
    elif port_held_by_other; then
        die "port $port is listening but this script has no PID file for it: not ours"
    fi
    printf 'port=%s pid=%s alive=%s boot_completed=%s serial=%s\n' \
        "$port" "${pid:-}" "$alive" "${booted:-}" "$serial"
}

case "$command" in
    create) cmd_create ;;
    start) cmd_start ;;
    wait) cmd_wait ;;
    stop) cmd_stop ;;
    status) cmd_status ;;
esac
