#!/usr/bin/env bash
# android/tools/ci-emulator.sh — the `android` workflow's headless smoke: the
# WebView app's RELEASE APK, installed and driven through its first user path.
#
#   android/tools/ci-emulator.sh --apk neezer.apk --server-log /tmp/server.log \
#       --server-port 5722 [--port 5590]
#
# The `android` app is a thin WebView around the SPA (`<server>/app/`), and the
# `apk` job only proved its structure: an archive, a signature, 4-byte alignment,
# a package id. It was never INSTALLED. A crash at the first Activity — a
# missing resource, a theme that no longer resolves, a bad `dataExtractionRules`
# — shipped on a green run, the class of defect the mobile app shipped twice
# (`v1.6.5` verifier, `v1.6.6` cover) while every job was green.
#
# What it proves, on the file `apk` published:
#   1. the APK installs and its LAUNCHER (SetupActivity) paints « Adresse du
#      serveur » — the first screen a person sees on a fresh install;
#   2. entering `http://10.0.2.2:<server>` and tapping « Se connecter » really
#      probes the server (any HTTP answer, SetupActivity.probe) and proceeds:
#      MainActivity becomes the resumed activity;
#   3. the WebView then fetches the SPA, proved at the SERVER, not in the UI: the
#      access log carries a request for an `/app/…js|css` bundle (the Java probe
#      only asks for `/app/`, so a bundle request can only be the WebView);
#   4. the app process is alive and the crash buffer is empty.
#
# The device lifecycle is mobile/tools/emu.sh (the ONE way a lot starts an
# emulator: no window, silent, its own port and PID, never the person's phone).
# Reusing it from `android/` is deliberate: a copied set of emulator flags gets
# one of those rules wrong eventually, and this tree has already paid for each.
#
# Not proved here: the SPA's own screens. They are `webapp.yaml`'s benches (the
# SPA in headless Chromium) and `mobile`'s release journey (a real signed-in
# app). This gate is the WebView APK's launch-and-connect path, which none of
# those can see.

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
readonly package="org.nsupysonic.app"

apk=""
server_log=""
port="${NEEZER_EMU_PORT:-5590}"
server_port=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --apk) apk="$2"; shift 2 ;;
        --apk=*) apk="${1#*=}"; shift ;;
        --server-log) server_log="$2"; shift 2 ;;
        --server-log=*) server_log="${1#*=}"; shift ;;
        --port) port="$2"; shift 2 ;;
        --port=*) port="${1#*=}"; shift ;;
        --server-port) server_port="$2"; shift 2 ;;
        --server-port=*) server_port="${1#*=}"; shift ;;
        -h | --help) sed -n '2,41p' "$0"; exit 0 ;;
        *) printf 'ci-emulator.sh: unknown argument: %s\n' "$1" >&2; exit 2 ;;
    esac
done

[[ -n "$apk" && -f "$apk" ]] || { printf 'ci-emulator.sh: --apk must name a built APK\n' >&2; exit 2; }
[[ -n "$server_port" ]] || { printf 'ci-emulator.sh: --server-port is required\n' >&2; exit 2; }
readonly serial="emulator-$port"

sdk_root() {
    if [[ -n "${ANDROID_SDK_ROOT:-}" ]]; then printf '%s\n' "$ANDROID_SDK_ROOT"
    elif [[ -n "${ANDROID_HOME:-}" ]]; then printf '%s\n' "$ANDROID_HOME"
    else printf '%s\n' "$HOME/.local/share/android-sdk"; fi
}
adb() { "$(sdk_root)/platform-tools/adb" -s "$serial" "$@"; }
note() { printf 'ci-emulator.sh: %s\n' "$*" >&2; }
step() { printf '\n=== %s\n' "$*"; }

# Always stop through emu.sh, even when a check fails.
cleanup() {
    local code=$?
    "$repo/mobile/tools/emu.sh" stop --port "$port" >/dev/null 2>&1 || true
    exit "$code"
}
trap cleanup EXIT

# --- 1. the device -------------------------------------------------------

step "emulator $serial"
"$repo/mobile/tools/emu.sh" create
"$repo/mobile/tools/emu.sh" start --port "$port"
boot_line="$("$repo/mobile/tools/emu.sh" wait --port "$port")"
note "$boot_line"
printf 'boot_seconds=%s\n' "${boot_line##*seconds=}"

# The same determinism `mobile/tools/ci-emulator.sh` gives the native app: no
# ANR/crash dialog over the screen, no animation for uiautomator to wait on.
adb shell settings put global hide_error_dialogs 1 >/dev/null 2>&1 || true
adb shell settings put global window_animation_scale 0 >/dev/null 2>&1 || true
adb shell settings put global transition_animation_scale 0 >/dev/null 2>&1 || true
adb shell settings put global animator_duration_scale 0 >/dev/null 2>&1 || true
adb shell input keyevent KEYCODE_WAKEUP >/dev/null 2>&1 || true
adb shell wm dismiss-keyguard >/dev/null 2>&1 || true

# --- helpers -------------------------------------------------------------

dump_ui() {
    local attempt
    for attempt in 1 2 3 4 5; do
        if adb shell uiautomator dump /sdcard/nz-ui.xml >/dev/null 2>&1; then
            adb shell cat /sdcard/nz-ui.xml 2>/dev/null
            return 0
        fi
    done
    return 1
}
# The bounds of the node carrying `attribute` (a resource-id or a text snippet).
bounds_of() {
    local dump="$1" needle="$2"
    printf '%s' "$dump" | tr '>' '\n' | grep -F "$needle" | tail -1 |
        sed -n 's/.*bounds="\[\([0-9]*\),\([0-9]*\)\]\[\([0-9]*\),\([0-9]*\)\]".*/\1 \2 \3 \4/p'
}
tap_bounds() {
    local x1 y1 x2 y2
    read -r x1 y1 x2 y2 <<<"$1"
    adb shell input tap "$(((x1 + x2) / 2))" "$(((y1 + y2) / 2))" >/dev/null
}
await_text() {
    local needle="$1" budget="${2:-10}" started=$SECONDS
    while (( SECONDS - started < budget )); do
        if dump_ui | grep -qF -- "text=\"$needle\""; then return 0; fi
    done
    return 1
}
app_pid() { adb shell pidof "$package" 2>/dev/null | tr -d '\r'; }
crash_cause() {
    adb shell logcat -d -b crash 2>/dev/null |
        grep -E "FATAL EXCEPTION|ANR in $package|UnsatisfiedLinkError|dlopen failed" | head -3 || true
}
resumed_activity() {
    adb shell dumpsys activity activities 2>/dev/null |
        grep -E "mResumedActivity|topResumedActivity" | grep -F "$package" | head -1 || true
}

# --- 2. install and launch ------------------------------------------------

step "install"
note "installing $apk ($(stat -c %s "$apk") bytes)"
adb install -r -g "$apk" 2>&1 | tail -2
launcher="$(adb shell cmd package resolve-activity --brief -a android.intent.action.MAIN \
    -c android.intent.category.LAUNCHER -p "$package" | tr -d '\r' | tail -1)"
[[ "$launcher" == "$package/"* ]] || { note "the launcher of $package does not resolve ($launcher)"; exit 1; }
note "launcher: $launcher"

# A FIRST run, on whatever user-data the AVD kept: a configured app opens
# MainActivity directly and the setup screen would never appear. `pm clear` is
# the documented "fresh install" state (and drops the `-g` grants, which this
# smoke does not need).
adb shell pm clear "$package" >/dev/null

step "launch: the first screen is the setup screen"
adb shell am force-stop "$package" >/dev/null 2>&1 || true
adb shell am start -W -n "$launcher" >/dev/null 2>&1 || true
# `setup_title` is a real TextView (the host field only carries a hint, which a
# dump does not always show): « Connexion au serveur » is the screen's own words.
if ! await_text "Connexion au serveur" 30; then
    note "the setup screen never painted « Connexion au serveur »"
    cause="$(crash_cause)"; [[ -n "$cause" ]] && note "cause : $cause"
    dump_ui | head -c 2000 >&2
    exit 1
fi
printf 'setup_visible=1 app_pid=%s\n' "$(app_pid)"

# --- 3. the first user action: point it at the server and connect ---------

# `Prefs.buildBaseUrl` adds the scheme when the field has none and the port is a
# separate field: `http://10.0.2.2` + the port is the URL this drives.
host_url="http://10.0.2.2:$server_port"

step "connect: $host_url"
dump="$(dump_ui)"
field="$(bounds_of "$dump" "resource-id=\"$package:id/field_host\"")"
[[ -n "$field" ]] || { note "no field_host on the setup screen"; dump_ui | head -c 2000 >&2; exit 1; }
tap_bounds "$field"
adb shell input text "$host_url" >/dev/null
# The IME hides on the first BACK; the activity does not (Android routes BACK to
# the keyboard while it is up), so this is not a way to leave the screen.
adb shell input keyevent 4 >/dev/null
dump="$(dump_ui)"
button="$(bounds_of "$dump" "resource-id=\"$package:id/btn_connect\"")"
[[ -n "$button" ]] || { note "no btn_connect on the setup screen"; dump_ui | head -c 2000 >&2; exit 1; }
tap_bounds "$button"

started=$SECONDS
resumed=""
while (( SECONDS - started < 25 )); do
    resumed="$(resumed_activity)"
    [[ -n "$resumed" ]] && break
    # The probe answered "unreachable": a real misconfiguration of this gate,
    # never a screen to click through.
    if dump_ui | grep -qF "injoignable"; then
        note "the app says the server at $host_url is unreachable"
        dump_ui | head -c 2000 >&2
        exit 1
    fi
done
[[ -n "$resumed" ]] || {
    note "MainActivity never became the resumed activity after « Se connecter »"
    dump_ui | head -c 2000 >&2
    exit 1
}
printf 'connected=1 main_resumed=1 seconds=%s\n' "$((SECONDS - started))"

# --- 4. the WebView fetched the SPA, and the app is still alive ----------

step "the WebView loaded the SPA"
page=""
started=$SECONDS
while (( SECONDS - started < 20 )); do
    if [[ -n "$server_log" && -f "$server_log" ]] &&
        grep -qE '"GET /app/[^"]*\.(js|css)[^"]*"' "$server_log"; then
        page="1"
        break
    fi
done
if [[ -z "$page" ]]; then
    note "no /app bundle request in the server log: the WebView did not run the SPA"
    if [[ -n "$server_log" && -f "$server_log" ]]; then
        grep -E '"GET /app' "$server_log" | head -10 >&2 || true
    fi
    exit 1
fi
grep -E '"GET /app/' "$server_log" | head -5
printf 'spa_loaded=1\n'

pid="$(app_pid)"
[[ -n "$pid" ]] || { note "the app process is gone after loading the SPA"; exit 1; }
cause="$(crash_cause)"
[[ -z "$cause" ]] || { note "the crash buffer carries a fatal: $cause"; exit 1; }
printf 'android_smoke=1 setup=1 connect=1 spa=1 crash=0 app_pid=%s\n' "$pid"
