#!/usr/bin/env bash
# mobile/tools/coexist.sh — the two Android apps on ONE device.
#
#   mobile/tools/coexist.sh [--port 5620] [--old-apk <path>] [--native-apk <path>]
#
# What it proves (docs/agents/mobile.md, "Deux apps Android"):
#
#   1. the old APK (`org.nsupysonic.app`) installs, then the native one
#      (`fr.neezer.mobile`) installs BESIDE it — no
#      `INSTALL_FAILED_CONFLICTING_PROVIDER`, no version-code comparison;
#   2. both launch, both keep a live process, and both declare their media
#      playback service;
#   3. each app's OWN channel serves its OWN APK through the real routes
#      (`/api/android/apk` vs `/api/android/native-apk?abi=`), the downloaded
#      file is byte-identical to the source, and installing one never bumps the
#      other's `lastUpdateTime`;
#   4. uninstalling either app leaves the other installed and untouched.
#
# The server is local (`coexist-channel.py`) and points `[webapp] android_apk` /
# `android_native_apk` at the two APKs handed here: no GitHub release is read.
#
# Rules, all from emu.sh: headless, silent, our own port, our own PIDs; every
# adb call names `emulator-<port>`. State under ~/.cache, never /tmp.

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=mobile/tools/env.sh
. "$here/env.sh"

port=5620
old_apk=""
native_apk=""
old_update_apk=""
native_update_apk=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --port) port="$2"; shift 2 ;;
        --port=*) port="${1#*=}"; shift ;;
        --old-apk) old_apk="$2"; shift 2 ;;
        --old-apk=*) old_apk="${1#*=}"; shift ;;
        --native-apk) native_apk="$2"; shift 2 ;;
        --native-apk=*) native_apk="${1#*=}"; shift ;;
        # The files each channel SERVES. When omitted the installed files are
        # served, and the reinstall is a same-version proof; when the caller
        # passes a build with a higher versionCode, the install is a REAL update
        # and the other app's version/time must stay put.
        --old-update-apk) old_update_apk="$2"; shift 2 ;;
        --old-update-apk=*) old_update_apk="${1#*=}"; shift ;;
        --native-update-apk) native_update_apk="$2"; shift 2 ;;
        --native-update-apk=*) native_update_apk="${1#*=}"; shift ;;
        -h | --help) sed -n '2,30p' "$0"; exit 0 ;;
        *) printf 'coexist.sh: unknown argument: %s\n' "$1" >&2; exit 2 ;;
    esac
done

repo="$(cd "$here/../.." && pwd)"
state="${NEEZER_COEXIST_STATE:-$HOME/.cache/neezer-coexist}/$port"
rm -rf "$state"
mkdir -p "$state"

[[ -n "$old_apk" ]] || old_apk="$repo/android/app/build/outputs/apk/release/app-release.apk"
[[ -n "$native_apk" ]] || native_apk="$repo/mobile/android/app/build/outputs/apk/release/app-universal-release.apk"
[[ -n "$old_update_apk" ]] || old_update_apk="$old_apk"
[[ -n "$native_update_apk" ]] || native_update_apk="$native_apk"
for f in "$old_apk" "$native_apk" "$old_update_apk" "$native_update_apk"; do
    [[ -f "$f" ]] || { printf 'coexist.sh: no APK at %s\n' "$f" >&2; exit 1; }
done

serial="emulator-$port"
adb_dev() { "$(nz_adb)" -s "$serial" "$@"; }
note() { printf 'coexist.sh: %s\n' "$*" >&2; }
step() { printf '\n=== %s\n' "$*"; }
fail() { printf 'coexist.sh: FAIL: %s\n' "$*" >&2; exit 1; }

aapt2() {
    local sdk
    sdk="$(nz_sdk_root)"
    ls -d "$sdk"/build-tools/*/aapt2 2>/dev/null | sort -V | tail -1
}
AAPT2="$(aapt2)" || true
[[ -n "$AAPT2" ]] || fail "no aapt2 under the SDK"

package_of() {
    "$AAPT2" dump badging "$1" 2>/dev/null |
        sed -n "s/^package: name='\([^']*\)'.*/\1/p" | head -1
}

version_code() {
    adb_dev shell dumpsys package "$1" 2>/dev/null |
        sed -n 's/.*versionCode=\([0-9][0-9]*\).*/\1/p' | head -1
}

apk_version_code() {
    "$AAPT2" dump badging "$1" 2>/dev/null |
        sed -n "s/.*versionCode='\([0-9][0-9]*\)'.*/\1/p" | head -1
}

last_update() {
    adb_dev shell dumpsys package "$1" 2>/dev/null |
        sed -n 's/.*lastUpdateTime=\([^ ]*\).*/\1/p' | head -1
}

is_installed() {
    [[ -n "$(adb_dev shell pm path "$1" 2>/dev/null | tr -d '\r')" ]]
}

await_pid() {
    local package="$1" started=$SECONDS
    local pid
    while (( SECONDS - started < 15 )); do
        pid="$(adb_dev shell pidof "$package" 2>/dev/null | tr -d '\r')"
        [[ -n "$pid" ]] && { printf '%s' "$pid"; return 0; }
    done
    return 1
}

server_pid=""
cleanup() {
    local code=$?
    if [[ -n "$server_pid" ]]; then
        kill "$server_pid" 2>/dev/null || true
        wait "$server_pid" 2>/dev/null || true
    fi
    "$here/emu.sh" stop --port "$port" >/dev/null 2>&1 || true
    exit "$code"
}
trap cleanup EXIT

# --- 1. the device ---------------------------------------------------------

step "emulator $serial"
running=$(pgrep -fc "qemu-system-x86_64-headless.*-port [0-9]*" || true)
note "emulators already running: ${running:-0} (emu.sh allows two; this one needs a free slot)"
"$here/emu.sh" create
"$here/emu.sh" start --port "$port"
"$here/emu.sh" wait --port "$port"
adb_dev shell settings put global hide_error_dialogs 1 >/dev/null 2>&1 || true
adb_dev shell settings put global window_animation_scale 0 >/dev/null 2>&1 || true
adb_dev shell settings put global transition_animation_scale 0 >/dev/null 2>&1 || true
adb_dev shell settings put global animator_duration_scale 0 >/dev/null 2>&1 || true
adb_dev shell input keyevent KEYCODE_WAKEUP >/dev/null 2>&1 || true
adb_dev shell wm dismiss-keyguard >/dev/null 2>&1 || true

# --- 2. the two apps, one device ------------------------------------------

step "install the OLD app, then the NATIVE one"
old_pkg="$(package_of "$old_apk")"
native_pkg="$(package_of "$native_apk")"
[[ "$old_pkg" == "org.nsupysonic.app" ]] || fail "the old APK is '$old_pkg', not org.nsupysonic.app"
[[ "$native_pkg" == "fr.neezer.mobile" ]] || fail "the native APK is '$native_pkg', not fr.neezer.mobile"

# The AVD's base image may carry an app a previous lot installed (measured:
# `org.nsupysonic.app` at versionCode 2000001, from the install-over plan's own
# test). `-read-only` does not make the base userdata empty. Remove the two
# packages this test owns; no other app is touched.
adb_dev uninstall "$old_pkg" >/dev/null 2>&1 || true
adb_dev uninstall "$native_pkg" >/dev/null 2>&1 || true

note "old:    $old_apk ($(stat -c %s "$old_apk") B, $old_pkg, code $(apk_version_code "$old_apk"))"
note "native: $native_apk ($(stat -c %s "$native_apk") B, $native_pkg, code $(apk_version_code "$native_apk"))"
note "old channel serves:    $old_update_apk (code $(apk_version_code "$old_update_apk"))"
note "native channel serves: $native_update_apk (code $(apk_version_code "$native_update_apk"))"
install_apk() {
    local apk="$1" out
    out="$(adb_dev install -r -g "$apk" 2>&1)" || {
        printf '%s\n' "$out" >&2
        fail "adb install refused $apk"
    }
    note "$(printf '%s\n' "$out" | tail -1)"
}
install_apk "$old_apk"
is_installed "$old_pkg" || fail "the old app did not install"
install_apk "$native_apk"
is_installed "$native_pkg" || fail "the native app did not install beside the old one"
is_installed "$old_pkg" || fail "installing the native app removed the old one"

old_code="$(version_code "$old_pkg")"
native_code="$(version_code "$native_pkg")"
old_data="$(adb_dev shell dumpsys package "$old_pkg" | sed -n 's/.*dataDir=\([^ ]*\).*/\1/p' | head -1)"
native_data="$(adb_dev shell dumpsys package "$native_pkg" | sed -n 's/.*dataDir=\([^ ]*\).*/\1/p' | head -1)"
[[ -n "$old_data" && -n "$native_data" && "$old_data" != "$native_data" ]] ||
    fail "the two apps share a data directory ($old_data / $native_data)"
printf 'package=%s versionCode=%s dataDir=%s\n' "$old_pkg" "$old_code" "$old_data"
printf 'package=%s versionCode=%s dataDir=%s\n' "$native_pkg" "$native_code" "$native_data"

# --- 3. both launch and keep their media service --------------------------

step "launch both"
old_launcher="$(nz_launcher_component_of "$serial" "$old_pkg")"
native_launcher="$(nz_launcher_component "$serial")"
[[ "$old_launcher" == "$old_pkg/"* ]] || fail "the old app has no launcher ($old_launcher)"
[[ "$native_launcher" == "$native_pkg/"* ]] || fail "the native app has no launcher ($native_launcher)"
adb_dev shell am start -W -n "$old_launcher" >/dev/null
old_pid="$(await_pid "$old_pkg")" || fail "the old app never stayed alive"
adb_dev shell am start -W -n "$native_launcher" >/dev/null
native_pid="$(await_pid "$native_pkg")" || fail "the native app never stayed alive"
[[ -n "$(adb_dev shell pidof "$old_pkg" | tr -d '\r')" ]] ||
    fail "launching the native app killed the old one"
printf 'launched: old pid=%s native pid=%s\n' "$old_pid" "$native_pid"

# The capability behind "it plays": each app declares its own media playback
# service. Two packages, two services, two sessions — nothing is shared.
#
# Read from the INSTALLED APK, not from `dumpsys package` (which does not list
# services by default on API 35 and made this check fail on a correct package):
# `pm path` gives what the device really has, and aapt2 reads its manifest.
installed_manifest_has() {
    local package="$1" needle="$2" path tree
    path="$(adb_dev shell pm path "$package" | tr -d '\r' | sed 's/^package://' | head -1)"
    [[ -n "$path" ]] || return 1
    adb_dev pull "$path" "$state/installed-$package.apk" >/dev/null 2>&1 || return 1
    # `| grep -q` under `set -o pipefail` fails the PIPELINE when grep exits on
    # the match and aapt2 then gets SIGPIPE (measured: a correct installed
    # manifest reported as missing). Read the tree whole, then match.
    tree="$("$AAPT2" dump xmltree "$state/installed-$package.apk" --file AndroidManifest.xml 2>/dev/null)" ||
        return 1
    case "$tree" in
        *"$needle"*) return 0 ;;
        *) return 1 ;;
    esac
}
installed_manifest_has "$old_pkg" "org.nsupysonic.app.PlayerService" ||
    fail "the installed old app no longer declares its PlayerService"
installed_manifest_has "$native_pkg" "fr.neezer.mobile.playback.NzMediaService" ||
    fail "the installed native app does not declare NzMediaService"

# --- 4. each app updates from its OWN channel ------------------------------

step "two channels, one local server"
sport=$((port + 100))
server_log="$state/channel.log"
python_bin="${PYTHON:-}"
if [[ -z "$python_bin" ]]; then
    if [[ -x "$repo/.venv/bin/python" ]]; then
        python_bin="$repo/.venv/bin/python"
    else
        python_bin="$(command -v python3)"
    fi
fi
[[ -n "$python_bin" ]] || fail "no Python: set PYTHON to a venv with the project installed"
"$python_bin" "$here/coexist-channel.py" \
    --old-apk "$old_update_apk" \
    --native-apk "$native_update_apk" \
    --port "$sport" \
    --state-dir "$state/channel" >"$server_log" 2>&1 &
server_pid=$!
base="http://127.0.0.1:$sport"
for _ in $(seq 1 60); do
    curl -sf "$base/api/version" >/dev/null 2>&1 && break
    sleep 1
done
curl -sf "$base/api/version" >/dev/null || {
    tail -20 "$server_log" >&2 || true
    fail "the local channel server never answered on $base"
}
jar="$state/cookies.txt"
curl -sf -c "$jar" -H 'Content-Type: application/json' \
    -d '{"username":"bench","password":"Bench1"}' "$base/api/login" >/dev/null ||
    fail "the bench login failed"

dl_old="$state/neezer.apk"
dl_native="$state/neezer-native-x86_64.apk"
curl -sf -b "$jar" -D "$state/old.headers" -o "$dl_old" "$base/api/android/apk" ||
    fail "the old channel answered nothing"
curl -sf -b "$jar" -D "$state/native.headers" -o "$dl_native" \
    "$base/api/android/native-apk?abi=x86_64" ||
    fail "the native channel answered nothing"

[[ "$(package_of "$dl_old")" == "$old_pkg" ]] ||
    fail "the old channel served $(package_of "$dl_old")"
[[ "$(package_of "$dl_native")" == "$native_pkg" ]] ||
    fail "the native channel served $(package_of "$dl_native")"
cmp -s "$dl_old" "$old_update_apk" || fail "the old channel did not serve the file it was given"
cmp -s "$dl_native" "$native_update_apk" || fail "the native channel did not serve the file it was given"
grep -qi '^X-NS-Version: 9.0.0' "$state/old.headers" ||
    fail "the old channel named no version"
grep -qi '^X-NS-Version: 9.1.0' "$state/native.headers" ||
    fail "the native channel named no version"
printf 'old channel:    %s bytes, %s, X-NS-Version 9.0.0\n' "$(stat -c %s "$dl_old")" "$(package_of "$dl_old")"
printf 'native channel: %s bytes, %s, X-NS-Version 9.1.0\n' "$(stat -c %s "$dl_native")" "$(package_of "$dl_native")"
cmp -s "$dl_old" "$dl_native" && fail "the two channels served the same file"

step "install each channel's file over its own app"
old_target="$(apk_version_code "$dl_old")"
native_target="$(apk_version_code "$dl_native")"
(( old_target >= old_code )) || fail "the old channel's APK is older ($old_target < $old_code)"
(( native_target >= native_code )) || fail "the native channel's APK is older ($native_target < $native_code)"
native_before="$(last_update "$native_pkg")"
native_code_before="$(version_code "$native_pkg")"
install_apk "$dl_old"
is_installed "$old_pkg" || fail "the old app did not survive its own update"
[[ "$(version_code "$old_pkg")" == "$old_target" ]] ||
    fail "the old app did not move to the channel's versionCode $old_target"
[[ "$(last_update "$native_pkg")" == "$native_before" ]] ||
    fail "updating the old app changed the native app's lastUpdateTime"
[[ "$(version_code "$native_pkg")" == "$native_code_before" ]] ||
    fail "updating the old app changed the native app's versionCode"
printf 'old app updated: %s -> %s\n' "$old_code" "$old_target"

old_before="$(last_update "$old_pkg")"
old_code_before="$(version_code "$old_pkg")"
install_apk "$dl_native"
is_installed "$native_pkg" || fail "the native app did not survive its own update"
[[ "$(version_code "$native_pkg")" == "$native_target" ]] ||
    fail "the native app did not move to the channel's versionCode $native_target"
[[ "$(last_update "$old_pkg")" == "$old_before" ]] ||
    fail "updating the native app changed the old app's lastUpdateTime"
[[ "$(version_code "$old_pkg")" == "$old_code_before" ]] ||
    fail "updating the native app changed the old app's versionCode"
printf 'native app updated: %s -> %s\n' "$native_code" "$native_target"

# --- 5. uninstall one, the other stays -------------------------------------

step "uninstall the old app"
adb_dev uninstall "$old_pkg" >/dev/null
is_installed "$old_pkg" && fail "the old app is still installed after uninstall"
is_installed "$native_pkg" || fail "uninstalling the old app removed the native one"
[[ -n "$(adb_dev shell pm path "$native_pkg" | tr -d '\r')" ]] ||
    fail "the native package path is gone"
printf 'old app removed; native app still installed (%s)\n' "$(version_code "$native_pkg")"

step "reinstall the old app, then uninstall the native one"
install_apk "$dl_old" >/dev/null
is_installed "$old_pkg" || fail "the old app did not reinstall from its channel file"
adb_dev uninstall "$native_pkg" >/dev/null
is_installed "$native_pkg" && fail "the native app is still installed after uninstall"
is_installed "$old_pkg" || fail "uninstalling the native app removed the old one"
printf 'native app removed; old app still installed (%s)\n' "$(version_code "$old_pkg")"

# Leave the device with both, the state the rest of the acceptance reads.
install_apk "$dl_native" >/dev/null
is_installed "$old_pkg" && is_installed "$native_pkg" ||
    fail "the final state does not have both apps"

step "done"
printf 'coexist_ok=1 old=%s@%s native=%s@%s\n' \
    "$old_pkg" "$(version_code "$old_pkg")" \
    "$native_pkg" "$(version_code "$native_pkg")"
