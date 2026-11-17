#!/usr/bin/env bash
# mobile/tools/ci-emulator.sh — what .github/workflows/mobile.yaml's `emulator`
# job runs: boot the AVD headless, install the app, walk its shell, then run every
# module's instrumented tests.
#
#   mobile/tools/ci-emulator.sh [--port 5580] [--apk <path>]
#
# Why a script and not twenty lines of YAML: the same walk has to be runnable on
# this machine (a lot checking its own screen) and on the runner, and the rules it
# encodes are the ones that keep the person's display and phone out of it:
#
#  - the emulator starts through `emu.sh` (no window, no sound, our own port, our
#    own PID file) and is stopped through `emu.sh` at the end, whatever happened;
#  - every `adb` call names `emulator-<port>`;
#  - "the screen opened" is proved from the accessibility tree (the same nodes
#    TalkBack reads), never from a screenshot's pixels and never from a sleep;
#  - a route this skeleton cannot open YET is counted and printed, not silently
#    skipped: the numbers move as the lots land, and a door that stops opening
#    fails the job.
#
# What it measures and prints (a CI log is the only place these numbers are ever
# read): boot seconds, the shell's first screen, the doors opened / expected,
# routes reached through an app link, and the instrumented-test modules found.

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=mobile/tools/env.sh
. "$here/env.sh"

port="${NEEZER_EMU_PORT:-5580}"
apk=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --port) port="$2"; shift 2 ;;
        --port=*) port="${1#*=}"; shift ;;
        --apk) apk="$2"; shift 2 ;;
        --apk=*) apk="${1#*=}"; shift ;;
        -h | --help) sed -n '2,30p' "$0"; exit 0 ;;
        *) printf 'ci-emulator.sh: unknown argument: %s\n' "$1" >&2; exit 2 ;;
    esac
done

serial="emulator-$port"
adb_dev() { "$(nz_adb)" -s "$serial" "$@"; }
note() { printf 'ci-emulator.sh: %s\n' "$*" >&2; }
step() { printf '\n=== %s\n' "$*"; }

# Everything is stopped through emu.sh, even when a check fails: a runner that
# keeps a 3 GB qemu alive for its next job is a slow runner.
cleanup() {
    local code=$?
    "$here/emu.sh" stop --port "$port" >/dev/null 2>&1 || true
    exit "$code"
}
trap cleanup EXIT

# --- 1. the device -------------------------------------------------------

step "emulator $serial"
"$here/emu.sh" create
"$here/emu.sh" start --port "$port"
boot_line="$("$here/emu.sh" wait --port "$port")"
note "$boot_line"
boot_seconds="${boot_line##*seconds=}"
printf 'boot_seconds=%s\n' "$boot_seconds"

# The two facts the harness exists to guarantee, read from the process itself.
pid="$(tr -dc '0-9' <"${NEEZER_EMU_STATE:-$HOME/.cache/neezer-emu}/$port.pid")"
cmdline="$(tr '\0' '\n' <"/proc/$pid/cmdline")"
grep -qx -- '-no-window' <<<"$cmdline" || {
    printf 'ci-emulator.sh: the emulator has a window (%s)\n' "$cmdline" >&2
    exit 1
}
if ! grep -qx -- '-no-audio' <<<"$cmdline" && ! (grep -qx -- '-audio' <<<"$cmdline" && grep -qx -- 'none' <<<"$cmdline"); then
    printf 'ci-emulator.sh: the emulator is not silent (%s)\n' "$cmdline" >&2
    exit 1
fi
printf 'emulator_rss_bytes=%s\n' "$(awk '/VmRSS/ {print $2 * 1024}' "/proc/$pid/status")"

# --- 2. the app ----------------------------------------------------------

step "install"
if [[ -z "$apk" ]]; then
    for candidate in \
        "$(nz_android_project)/app/build/outputs/apk/release/app-release.apk" \
        "$(nz_android_project)/app/build/outputs/apk/benchmark/app-benchmark.apk"; do
        [[ -f "$candidate" ]] && apk="$candidate" && break
    done
fi
if [[ -z "$apk" || ! -f "$apk" ]]; then
    note "no APK built yet: assembling the release one (the workflow hands it over as an artifact)"
    nz_gradle_android :app:assembleRelease >/dev/null
    apk="$(nz_android_project)/app/build/outputs/apk/release/app-release.apk"
fi
[[ -f "$apk" ]] || {
    note "no APK at $apk"
    exit 1
}
note "installing $apk ($(stat -c %s "$apk") bytes)"
adb_dev install -r -g "$apk" 2>&1 | tail -2
nz_launcher_component "$serial" | grep -q "org.nsupysonic.app/" || {
    note "org.nsupysonic.app does not resolve its launcher after the install"
    exit 1
}

# --- 3. the shell's walk -------------------------------------------------

# The emulator is a qemu process on a software rasteriser: if it dies mid-walk,
# every later adb call answers "device not found" and the log would blame the
# wrong thing. Each step therefore proves the device is still ours, and a death
# prints the emulator's own last lines.
check_alive() {
    local where="$1"
    if ! kill -0 "$pid" 2>/dev/null; then
        note "the emulator died during $where; last log lines (${NEEZER_EMU_STATE:-$HOME/.cache/neezer-emu}/$port.log):"
        tail -n 30 "${NEEZER_EMU_STATE:-$HOME/.cache/neezer-emu}/$port.log" >&2 || true
        exit 1
    fi
    if [[ "$(adb_dev get-state 2>/dev/null || true)" != "device" ]]; then
        note "$serial is not visible to adb during $where"
        exit 1
    fi
}

# The app's own text, never a screenshot: `dump` is the accessibility tree, one
# long XML line. `uiautomator dump` refuses while the UI is animating (it waits
# for idle and gives up), so a dump is retried — a tab's ripple is not a failure.
dump_ui() {
    local attempt
    for attempt in 1 2 3 4 5; do
        if adb_dev shell uiautomator dump /sdcard/nz-ui.xml >/dev/null 2>&1; then
            adb_dev shell cat /sdcard/nz-ui.xml 2>/dev/null
            return 0
        fi
    done
    return 1
}

# Waits until a node whose text contains <needle> exists, or fails after 10 s.
await_text() {
    local needle="$1" started=$SECONDS
    while (( SECONDS - started < 10 )); do
        if dump_ui | grep -qF -- "$needle"; then
            return 0
        fi
    done
    return 1
}

# Taps the node whose text is exactly <label>: its bounds come from the
# accessibility tree, so the tap lands on the control and not on a coordinate
# that happened to be right on this screen size. The LAST match is taken on
# purpose: a tab's label appears twice (the top bar's title, then the bar's own
# item) and the clickable one is the bar's — tapping the title would do nothing
# and every door after the first would "fail" for the wrong reason.
tap_text() {
    local label="$1" bounds cx cy
    bounds="$(dump_ui | tr '>' '\n' | grep -E "text=\"$label\"" |
        sed -n 's/.*bounds="\[\([0-9]*\),\([0-9]*\)\]\[\([0-9]*\),\([0-9]*\)\]".*/\1 \2 \3 \4/p' | tail -1)"
    [[ -n "$bounds" ]] || return 1
    read -r x1 y1 x2 y2 <<<"$bounds"
    cx=$(((x1 + x2) / 2))
    cy=$(((y1 + y2) / 2))
    adb_dev shell input tap "$cx" "$cy" >/dev/null
}

launch_app() {
    adb_dev shell am force-stop org.nsupysonic.app >/dev/null 2>&1 || true
    adb_dev shell am start -W -n "$(nz_launcher_component "$serial")" >/dev/null
    await_text "Accueil"
}

step "smoke: launch"
launch_app || {
    note "the shell never painted « Accueil »"
    dump_ui | head -c 2000 >&2
    exit 1
}
printf 'shell_visible=1\n'
check_alive "the launch"

# Each door is proved by the text ONLY that screen draws (its placeholder's
# body), so a tab that opens the previous screen fails here. The bodies are
# MB0's and disappear as the lots fill them, which is why a door that fails says
# what it expected.
#
# No apostrophe in a needle: uiautomator writes the tree as XML ATTRIBUTES and
# escapes `'`, so a needle with one can never match (« d&#39;un podcast »).
step "smoke: the shell's doors"
doors=0
doors_failed=0
check_door() {
    local tab="$1" needle="$2"
    if ! tap_text "$tab"; then
        note "no node « $tab » to tap"
        doors_failed=$((doors_failed + 1))
        return
    fi
    if await_text "$needle"; then
        printf 'door=%s opened=1\n' "$tab"
        doors=$((doors + 1))
    else
        printf 'door=%s opened=0 expected="%s"\n' "$tab" "$needle"
        doors_failed=$((doors_failed + 1))
    fi
    check_alive "the $tab door"
}
check_door "Rechercher" "Titres, albums, artistes, playlists et podcasts"
check_door "Bibliothèque" "Playlists et dossiers, favoris"
check_door "Podcasts" "Les abonnements, la recherche"
check_door "Accueil" "Le hero, la reprise, les humeurs"

# Réglages, and the one line that proves the whole chain Kotlin -> JNA -> Rust:
# the version comes from `nz_version()`, not from a Kotlin constant.
if tap_text "Réglages" && await_text "Choisis une section." && tap_text "Aide" &&
    await_text "Version du noyau (Rust) :"; then
    printf 'door=Réglages opened=1 rust_version=1\n'
    doors=$((doors + 1))
else
    printf 'door=Réglages opened=0\n'
    doors_failed=$((doors_failed + 1))
fi
check_alive "the Réglages door"

printf 'doors_opened=%s doors_failed=%s doors_expected=5\n' "$doors" "$doors_failed"

# The routes the shell itself has no door for yet. They are opened by an APP
# LINK (the share sheet's text, a `neezer://` view): MB2's `resolve_link` is what
# turns one into a destination, and until it does this count is 0 — printed, not
# hidden, so the day it starts working (or stops) is visible in the log.
#
# Each probe returns to the home screen first, so "the route opened" cannot mean
# "the previous screen was still on top". Five probes only: a smoke, not a bench.
step "smoke: routes through an app link"
links_probed=0
links_opened=0
for probe in \
    "/app/#/artist/1#Artiste" \
    "/app/#/album/1#Album" \
    "/app/#/playlist/1#Playlist" \
    "/app/#/recap#Récap" \
    "/app/#/genres#Genres"; do
    url="https://neezer.example${probe%%#*}"
    needle="${probe##*#}"
    links_probed=$((links_probed + 1))
    launch_app >/dev/null 2>&1 || true
    adb_dev shell am start -a android.intent.action.SEND -t text/plain \
        --es android.intent.extra.TEXT "$url" >/dev/null 2>&1 || true
    if await_text "$needle"; then
        printf 'link=%s opened=1\n' "${probe%%#*}"
        links_opened=$((links_opened + 1))
    else
        printf 'link=%s opened=0 expected="%s"\n' "${probe%%#*}" "$needle"
    fi
    check_alive "the link probe ${probe%%#*}"
done
printf 'links_opened=%s links_probed=%s (resolve_link is MB2 own: 0 today is expected, and a door that stops opening still fails above)\n' \
    "$links_opened" "$links_probed"

if (( doors_failed > 0 )); then
    note "$doors_failed door(s) of the shell did not open"
    exit 1
fi

# --- 4. every module's instrumented tests --------------------------------

# Later lots put their instrumented tests in THEIR module (`src/androidTest`),
# never in :app (docs/agents/mobile.md, MQ1). Discovering them by directory is
# what makes "over every module" true without naming a list here that goes stale.
step "instrumented tests"
mapfile -t test_modules < <(
    cd "$(nz_android_project)"
    for dir in */src/androidTest feature/*/src/androidTest; do
        [[ -d "$dir" ]] || continue
        module="${dir%%/src/androidTest}"
        module="${module#feature/}"
        [[ "$dir" == feature/* ]] && module=":feature:$module" || module=":$module"
        printf '%s\n' "$module"
    done | sort -u
)
if (( ${#test_modules[@]} == 0 )); then
    # Worth saying out loud, and worth its own line in the log: no module declares
    # an androidTest dependency yet, so a test placed in `src/androidTest` today
    # would not compile. The catalog (`libs.versions.toml`) is the skeleton lot's
    # file; MQ1 reported it rather than editing it.
    printf 'instrumented_modules=0 (no module has src/androidTest yet)\n'
else
    printf 'instrumented_modules=%s (%s)\n' "${#test_modules[@]}" "${test_modules[*]}"
    tasks=()
    for module in "${test_modules[@]}"; do
        tasks+=("$module:connectedDebugAndroidTest")
    done
    # Only OUR device: a connected task otherwise installs and runs on every
    # emulator adb sees, including another lot's.
    ANDROID_SERIAL="$serial" nz_gradle_android "${tasks[@]}"
fi

step "done"
printf 'smoke_ok=1 doors_opened=%s boot_seconds=%s\n' "$doors" "$boot_seconds"
