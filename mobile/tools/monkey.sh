#!/usr/bin/env bash
# mobile/tools/monkey.sh — the NIGHTLY CRAWLER: Android Monkey on the signed-in
# RELEASE APK, failing on any crash, ANR or dead process (docs/agents/mobile.md,
# "Tests and emulation"; plan `plan-mobile-tests`, lot T2).
#
#   mobile/tools/monkey.sh --serial emulator-5580 --events 3000 --seed 20261125
#   mobile/tools/monkey.sh --serial emulator-5580 --apk /tmp/neezer-native-1.6.6.apk
#
# T1's `release-journey.sh` is the deterministic GATE: eight doors, on the
# shipped APK. This is the NET under it — 3 000 pseudo-random events at 250 ms,
# seeded, inside `fr.neezer.mobile` only, after a real sign-in — so every crash
# the scripted flows never reach lands somewhere.
#
# What it guarantees, each learned on a real run:
#
#  - the emulator is started and stopped through `emu.sh` (never a stray qemu,
#    never a window, never the person's phone): every `adb` call names the
#    serial it was given, and the cleanup is a trap, like `ci-emulator.sh`;
#  - the APK under the crawl is the RELEASE one — minified by R8, signed with
#    the stable key, the file a person installs — and it is SIGNED IN first
#    (`release-signin.sh`, the home painted with real covers): a crawler that
#    starts on the setup screen explores one screen;
#  - a crash is a FAILURE, never a result: a record in the crash buffer that
#    names our process, `ANR in fr.neezer.mobile`, monkey's own `// CRASH:` /
#    `// NOT RESPONDING:`, or a `pidof` that came back empty all fail the run,
#    and the cause's first sentence is printed BEFORE the summary — `crashes=1`
#    next to the reason, never a count alone;
#  - the seed is honoured (`-s`), so a red nightly is reproducible instead of a
#    lottery, and two runs of the same seed inject the same number of events;
#  - the crawl stays inside the app: `--pct-syskeys 0 --pct-appswitch 0`, and
#    the resumed activity is read back afterwards (`dumpsys activity`).
#
# Prints, on success:
#   monkey=ok events=3000 crashes=0 anrs=0 app_alive=1 seconds=<n>
# On a crash/ANR/dead process it prints the cause first, then
#   monkey=fail events=<n> crashes=<n> anrs=<n> app_alive=<0|1> seconds=<n>
# and exits 1. `NEEZER_MONKEY_EVIDENCE_DIR` (default: a fresh `mktemp -d`,
# printed) keeps `crash.log`, `logcat.log`, `screencap.png`, `monkey.log` and
# `signin.log`; the CI job uploads them whatever happened, so a red nightly names
# the screen and the stack instead of a number.
#
# `--apk <path>` installs that file instead of the built universal release APK:
# it is how the plan's crash proof runs the published `v1.6.6` (`NoSuchMethodError`
# on the home's first cover) through THIS script, and how a human re-checks a
# shipped file. Without it the default is `:app:assembleRelease`'s output.

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=mobile/tools/env.sh
. "$here/env.sh"

serial=""
events=3000
seed=20261125
apk=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --serial) serial="$2"; shift 2 ;;
        --serial=*) serial="${1#*=}"; shift ;;
        --events) events="$2"; shift 2 ;;
        --events=*) events="${1#*=}"; shift ;;
        --seed) seed="$2"; shift 2 ;;
        --seed=*) seed="${1#*=}"; shift ;;
        --apk) apk="$2"; shift 2 ;;
        --apk=*) apk="${1#*=}"; shift ;;
        -h | --help) sed -n '2,52p' "$0"; exit 0 ;;
        *) printf 'monkey.sh: unknown argument: %s\n' "$1" >&2; exit 2 ;;
    esac
done
[[ -n "$serial" ]] || { printf 'monkey.sh: --serial is required\n' >&2; exit 2; }
[[ "$events" =~ ^[1-9][0-9]*$ ]] || { printf 'monkey.sh: --events must be a positive integer\n' >&2; exit 2; }
[[ "$seed" =~ ^[0-9]+$ ]] || { printf 'monkey.sh: --seed must be an integer\n' >&2; exit 2; }

# The tree, the pid and the crash cause live once, in ui.sh: this script walks
# no screen, but "is the app alive", "why did it die" and "what was on screen"
# must have ONE answer for the journey and the crawler.
# shellcheck source=mobile/tools/ui.sh
. "$here/ui.sh"

note() { printf 'monkey.sh: %s\n' "$*" >&2; }
step() { printf '\n=== %s\n' "$*"; }
die() { note "$*"; exit 2; }

evidence="${NEEZER_MONKEY_EVIDENCE_DIR:-}"
if [[ -z "$evidence" ]]; then
    evidence="$(mktemp -d "${TMPDIR:-/tmp}/neezer-monkey.XXXXXX")"
fi
mkdir -p "$evidence"
note "evidence: $evidence"

started=$SECONDS
port=""
if [[ "$serial" == emulator-* ]]; then
    port="${serial#emulator-}"
fi

# Everything is stopped through `emu.sh`, whatever happened — and only the
# emulator THIS script started (that is the whole point of emu.sh's PID file).
cleanup() {
    local code=$?
    if [[ -n "$port" ]]; then
        "$here/emu.sh" stop --port "$port" >/dev/null 2>&1 || true
    fi
    exit "$code"
}
trap cleanup EXIT

# --- the device -------------------------------------------------------------

if [[ -n "$port" ]]; then
    step "emulator $serial"
    "$here/emu.sh" create
    "$here/emu.sh" start --port "$port"
    "$here/emu.sh" wait --port "$port"
fi
prepare_device
check_alive "the boot"

# --- the app ----------------------------------------------------------------

resolve_apk() {
    [[ -n "$apk" ]] && return 0
    local candidate
    for candidate in \
        "$(nz_android_project)/app/build/outputs/apk/release/app-universal-release.apk" \
        "$(nz_android_project)/app/build/outputs/apk/release/app-release.apk"; do
        if [[ -f "$candidate" ]]; then
            apk="$candidate"
            return 0
        fi
    done
    return 1
}
resolve_apk || die "no release APK: the workflow hands it over as an artifact; locally, build it with \`gradle -p mobile/android :app:assembleRelease\`"
[[ -f "$apk" ]] || die "no APK at $apk"

step "install $(basename "$apk")"
note "$(stat -c %s "$apk") bytes"
adb_dev install -r -g "$apk" 2>&1 | tail -2
launcher="$(nz_launcher_component "$serial")"
[[ "$launcher" == "$ui_package/"* ]] ||
    die "fr.neezer.mobile does not resolve its launcher after installing $apk (got «$launcher»)"

# --- the sign-in (the crawl's precondition) ---------------------------------

# A crawler on a freshly installed app explores the setup screen and nothing
# else. `release-signin.sh` is T1's two phases (the platform trust store, then
# HTTPS + the home with `covers_served ≥ 1`) on this same RELEASE APK; its bench
# server is its own and stops with it, so the crawl then also walks the offline
# states a bad mobile link produces — a state a person meets, and one a crash
# can hide in. What a failed sign-in means is NOT a crawler result: it is a
# startup/sign-in failure, and it is reported as one.
step "release sign-in"
adb_dev logcat -c >/dev/null 2>&1 || true
signin_ok=1
"$here/release-signin.sh" --serial "$serial" >"$evidence/signin.log" 2>&1 || signin_ok=0
if (( signin_ok == 1 )); then
    grep -E '^release_(tls_trusted|signin)=' "$evidence/signin.log" || true
else
    note "the release sign-in FAILED: this is a startup/sign-in result, not a crawler result"
    sed 's/^/  /' "$evidence/signin.log" >&2
fi

# --- the evidence -----------------------------------------------------------

collect_evidence() {
    adb_dev logcat -d -b crash >"$evidence/crash.log" 2>/dev/null || true
    adb_dev logcat -d -b main >"$evidence/logcat.log" 2>/dev/null || true
    adb_dev exec-out screencap -p >"$evidence/screencap.png" 2>/dev/null || true
    adb_dev shell dumpsys activity activities >"$evidence/activity.txt" 2>/dev/null || true
    : >"$evidence/app-crash.log"
    # Only the crash records that name OUR process: the guest's crash buffer
    # also collects SystemUI's and SwiftShader's, and those are not this app's
    # result (the same rule ui.sh's `app_crash_cause` encodes).
    awk '
        function flush() {
            if (buf != "" && buf ~ /fr\.neezer\.mobile/) printf "%s", buf
            buf = ""
        }
        /^--------- beginning of/ { flush(); next }
        { buf = buf $0 "\n" }
        END { flush() }
    ' "$evidence/crash.log" >"$evidence/app-crash.log"
}

# The numbers the summary prints. `crashes` is the greater of the device's own
# crash records and monkey's report of the same events: either source missing
# one is exactly why both are read.
analyse() {
    crashes="$(grep -cE 'FATAL EXCEPTION|Fatal signal|UnsatisfiedLinkError|dlopen failed' \
        "$evidence/app-crash.log" 2>/dev/null || true)"
    local monkey_crashes
    monkey_crashes="$(grep -cE '^// (CRASH|NOT RESPONDING): fr\.neezer\.mobile' \
        "$evidence/monkey.log" 2>/dev/null || true)"
    anrs="$(grep -c 'ANR in fr\.neezer\.mobile' "$evidence/logcat.log" 2>/dev/null || true)"
    [[ "$monkey_crashes" =~ ^[0-9]+$ ]] || monkey_crashes=0
    [[ "$crashes" =~ ^[0-9]+$ ]] || crashes=0
    [[ "$anrs" =~ ^[0-9]+$ ]] || anrs=0
    if (( monkey_crashes > crashes )); then
        crashes=$monkey_crashes
    fi
    injected="$(sed -n 's/^Events injected: *\([0-9][0-9]*\).*/\1/p' "$evidence/monkey.log" 2>/dev/null | tail -1)"
    [[ -n "$injected" ]] || injected="$events"
}

# The first sentence of the cause, which a job log is read for. The exception
# line, not the `FATAL EXCEPTION: main` header: "java.lang.NoSuchMethodError…"
# names what broke, the header names that something did.
first_cause() {
    local line
    line="$(grep -m1 -E '(java|kotlin|android|dalvik)\.[A-Za-z0-9_$.]+(Exception|Error)|ANR in fr\.neezer\.mobile|UnsatisfiedLinkError|dlopen failed' \
        "$evidence/app-crash.log" 2>/dev/null || true)"
    if [[ -z "$line" ]]; then
        line="$(grep -m1 'ANR in fr\.neezer\.mobile' "$evidence/logcat.log" 2>/dev/null || true)"
    fi
    if [[ -z "$line" ]] && (( ${monkey_status:-0} != 0 )); then
        line="$(tail -1 "$evidence/monkey.log" 2>/dev/null || true)"
    fi
    if [[ -z "$line" ]]; then
        line="$(app_crash_cause 2>/dev/null | head -1 || true)"
    fi
    printf '%s\n' "$line"
}

report() {
    local ok="$1" alive=0 cause
    [[ -n "$(app_pid)" ]] && alive=1
    if (( ok == 0 )); then
        cause="$(first_cause)"
        printf 'monkey: cause: %s\n' "${cause:-unknown (the process is gone and no buffer names it)}"
        # The record itself, bounded: the stack a person needs is in the log.
        sed -n '1,24p' "$evidence/app-crash.log" | sed 's/^/  /' >&2 || true
    fi
    printf 'monkey=%s events=%s crashes=%s anrs=%s app_alive=%s seconds=%s\n' \
        "$([[ $ok == 1 ]] && printf ok || printf fail)" \
        "$injected" "$crashes" "$anrs" "$alive" "$((SECONDS - started))"
}

if (( signin_ok == 0 )); then
    collect_evidence
    monkey_status=0
    analyse
    report 0
    exit 1
fi

# --- the crawl --------------------------------------------------------------

step "clear the buffers, then monkey: $events events, seed $seed, throttle 250 ms"
# `logcat -c` clears every buffer, so only the crawl's lines are analysed.
adb_dev logcat -c >/dev/null 2>&1 || true
adb_dev shell command -v monkey >/dev/null 2>&1 ||
    die "the system image carries no monkey ($(nz_sdk_root))"

monkey_status=0
adb_dev shell monkey \
    -p "$ui_package" \
    --throttle 250 \
    --pct-syskeys 0 \
    --pct-appswitch 0 \
    --monitor-native-crashes \
    -s "$seed" "$events" >"$evidence/monkey.log" 2>&1 || monkey_status=$?
grep -m1 -E 'Events injected|Monkey aborted' "$evidence/monkey.log" | sed 's/^/  /' || true

step "analyse"
check_alive "the crawl"
collect_evidence
analyse

fail=0
if (( crashes > 0 || anrs > 0 || monkey_status != 0 )); then
    fail=1
fi
if [[ -z "$(app_pid)" ]]; then
    note "the process is gone after the crawl"
    fail=1
fi
# `--pct-syskeys 0 --pct-appswitch 0` is the promise that the crawl never left
# the app; the resumed activity is how that promise is checked, not assumed.
resumed="$(grep -m1 -E 'topResumedActivity|mResumedActivity' "$evidence/activity.txt" 2>/dev/null || true)"
if [[ -z "$resumed" ]]; then
    note "could not read the resumed activity from dumpsys"
elif [[ "$resumed" != *"$ui_package/"* ]]; then
    note "the crawl ended outside the app: $resumed"
    fail=1
fi

if (( monkey_status != 0 )); then
    note "monkey exited $monkey_status"
fi

report $(( 1 - fail ))
exit "$fail"
