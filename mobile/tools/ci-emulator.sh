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
# routes reached through an app link, the instrumented-test modules found, and
# whether the native library was handed over or built here.

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

# Are the native library and the generated Kotlin bindings ALREADY here?
#
# `mobile/rust/build-android.sh` produces exactly two things `:bridge` needs:
# `bridge/src/main/jniLibs/<abi>/libnz_ffi.so` for the ABIs the run built
# (`NZ_ABIS`: all three off a pull request, x86_64 alone on one),
# and the UniFFI Kotlin under `bridge/build/generated/uniffi`. The workflow's
# `apk` job builds them and hands them over as an artifact; without this test
# this job rebuilt the whole stack from cold inside the instrumented step —
# measured on run 44368850444: 12 min 30 s of cargo (18:48:36 → 19:01:03) while
# the emulator's four vCPUs waited, after installing the NDK and cargo-ndk a
# second time.
#
# A machine that has not run that job keeps the old path: the artifacts are
# absent, so `prepare_android_build` runs and Gradle's own `:bridge:buildRust`
# builds them, exactly as before. Nothing is skipped that was ever required —
# the SAME instrumented tasks run either way.
native_artifacts_present() {
    local abi generated
    generated="$(nz_android_project)/bridge/build/generated/uniffi"
    [[ -d "$generated" ]] || return 1
    [[ -n "$(find "$generated" -name '*.kt' -print -quit 2>/dev/null)" ]] || return 1
    for abi in ${NZ_ABIS:-arm64-v8a armeabi-v7a x86_64}; do
        [[ -f "$(nz_android_project)/bridge/src/main/jniLibs/$abi/libnz_ffi.so" ]] || return 1
    done
    return 0
}

# Bench servers this script started, so a failure still stops them. `perf-server.sh`
# owns its own pid file and is stopped through itself; `tone-server.py` is a bare
# Python process and is killed here.
bench_pids=()
stop_bench_servers() {
    local pid
    for pid in "${bench_pids[@]}"; do
        kill -TERM "$pid" 2>/dev/null || true
    done
    if [[ -n "${bench_perf_state:-}" ]]; then
        NEEZER_PERF_STATE="$bench_perf_state" "$here/perf-server.sh" stop >/dev/null 2>&1 || true
    fi
}

# Everything is stopped through emu.sh, even when a check fails: a runner that
# keeps a 3 GB qemu alive for its next job is a slow runner.
cleanup() {
    local code=$?
    stop_bench_servers
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

# A freshly booted guest on a busy machine pops "System UI isn't responding" on
# top of the app, and every later dump then describes the DIALOG, not the screen.
# Measured: one run of this very script failed "the shell never painted « Accueil »"
# with the tree showing `android:id/parentPanel` — a system dialog. Two settings,
# both standard for device tests, make the device deterministic:
#   hide_error_dialogs   no ANR/crash dialog ever covers the app;
#   0 animation scales   uiautomator waits for idle, and a ripple that never ends
#                        is a dump that never returns.
prepare_device() {
    adb_dev shell settings put global hide_error_dialogs 1 >/dev/null 2>&1 || true
    adb_dev shell settings put global window_animation_scale 0 >/dev/null 2>&1 || true
    adb_dev shell settings put global transition_animation_scale 0 >/dev/null 2>&1 || true
    adb_dev shell settings put global animator_duration_scale 0 >/dev/null 2>&1 || true
    adb_dev shell input keyevent KEYCODE_WAKEUP >/dev/null 2>&1 || true
    adb_dev shell wm dismiss-keyguard >/dev/null 2>&1 || true
}
prepare_device

# --- 2. the app ----------------------------------------------------------

step "install"
# The UNIVERSAL APK first: since « MQ2 — la taille » `assembleRelease` writes one
# APK per ABI plus `app-universal-release.apk` (the one that installs on the
# emulator's x86_64 as well as on a phone), and the two plain names below are
# what a tree from before that change still has.
if [[ -z "$apk" ]]; then
    for candidate in \
        "$(nz_android_project)/app/build/outputs/apk/release/app-universal-release.apk" \
        "$(nz_android_project)/app/build/outputs/apk/release/app-release.apk" \
        "$(nz_android_project)/app/build/outputs/apk/benchmark/app-universal-benchmark.apk" \
        "$(nz_android_project)/app/build/outputs/apk/benchmark/app-benchmark.apk"; do
        [[ -f "$candidate" ]] && apk="$candidate" && break
    done
fi
if [[ -z "$apk" || ! -f "$apk" ]]; then
    note "no APK built yet: assembling the release one (the workflow hands it over as an artifact)"
    if native_artifacts_present; then
        nz_gradle_android :app:assembleRelease -PskipRust >/dev/null
    else
        nz_gradle_android :app:assembleRelease >/dev/null
    fi
    apk="$(nz_android_project)/app/build/outputs/apk/release/app-universal-release.apk"
fi
[[ -f "$apk" ]] || {
    note "no APK at $apk"
    exit 1
}
note "installing $apk ($(stat -c %s "$apk") bytes)"
adb_dev install -r -g "$apk" 2>&1 | tail -2
nz_launcher_component "$serial" | grep -q "fr.neezer.mobile/" || {
    note "fr.neezer.mobile does not resolve its launcher after the install"
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

# Waits until a node whose text contains <needle> exists, or fails after the
# timeout (10 s for a tap's repaint, 30 s for a cold start on a loaded runner).
await_text_timeout() {
    local needle="$1" budget="${2:-10}" started=$SECONDS
    while (( SECONDS - started < budget )); do
        if dump_ui | grep -qF -- "$needle"; then
            return 0
        fi
    done
    return 1
}

await_text() { await_text_timeout "$1" 10; }

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

# Swipes up until <needle> is on screen (a menu longer than the window), then
# leaves it visible for tap_text. Bounded: five swipes, and each one is a gesture
# the device performed, never a sleep.
scroll_to_text() {
    local needle="$1" attempt
    for attempt in 1 2 3 4 5; do
        if dump_ui | grep -qF -- "text=\"$needle\""; then
            return 0
        fi
        adb_dev shell input swipe 540 1600 540 800 240 >/dev/null
    done
    dump_ui | grep -qF -- "text=\"$needle\""
}

# The app's own process, or nothing: a `am start` that returns 0 proves a task was
# created, never that the app is still alive two seconds later.
app_pid() {
    adb_dev shell pidof fr.neezer.mobile 2>/dev/null | tr -d '\r'
}

# WHY the app is not there. A native library that cannot be resolved, or a Kotlin
# exception at startup, leaves the screen black and the accessibility tree empty —
# the symptom this script used to report alone ("« Accueil » never painted"),
# which names nothing actionable. The crash buffer names the cause.
#
# The lesson behind it (measured 2026-11-18): `libnz_ffi.so` carried
# `NEEDED libc++_shared.so`, no APK shipped that file, and the whole app died at
# `System.loadLibrary` with `UnsatisfiedLinkError: dlopen failed: library
# "libc++_shared.so" not found`. `mobile/rust/build-android.sh` now refuses to
# produce such a library, and this prints the reason if one ever gets through.
app_crash_cause() {
    local crash
    crash="$(adb_dev logcat -d -b crash 2>/dev/null |
        grep -E "UnsatisfiedLinkError|FATAL EXCEPTION|dlopen failed" | head -3 || true)"
    if [[ -n "$crash" ]]; then
        printf '%s' "$crash"
        return 0
    fi
    if [[ -z "$(app_pid)" ]]; then
        printf 'le processus fr.neezer.mobile ne tourne pas et le tampon de plantage est vide'
    fi
}

launch_app() {
    local attempt
    for attempt in 1 2 3; do
        adb_dev shell am force-stop fr.neezer.mobile >/dev/null 2>&1 || true
        adb_dev shell am start -W -n "$(nz_launcher_component "$serial")" >/dev/null 2>&1 || true
        if await_text_timeout "Accueil" 30; then
            return 0
        fi
        # A dialog left over from an earlier step (or the app's own first-run
        # notice) is dismissed before the retry, never waited on.
        adb_dev shell input keyevent KEYCODE_BACK >/dev/null 2>&1 || true
    done
    return 1
}

step "smoke: launch"
if ! launch_app; then
    note "the shell never painted « Accueil »"
    cause="$(app_crash_cause)"
    [[ -n "$cause" ]] && note "cause : $cause"
    dump_ui | head -c 2000 >&2
    exit 1
fi
# The screen exists AND the app is still the app: a process that died a moment
# after painting would let every later door fail for the wrong reason.
pid_app="$(app_pid)"
[[ -n "$pid_app" ]] || {
    note "the app painted « Accueil » puis son processus a disparu"
    cause="$(app_crash_cause)"
    [[ -n "$cause" ]] && note "cause : $cause"
    exit 1
}
printf 'shell_visible=1 app_pid=%s\n' "$pid_app"
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
# M1 : une installation neuve ouvre l'écran serveur. C'est la porte qui manquait
# (l'audit : la route Setup était déclarée et jamais atteinte).
if await_text "Adresse du serveur"; then
    printf 'door=Serveur opened=1\n'
    doors=$((doors + 1))
else
    printf 'door=Serveur opened=0 expected="Adresse du serveur"\n'
    doors_failed=$((doors_failed + 1))
fi
# MB6 filled the search: its MB0 placeholder sentence is gone, and the screen's
# own marker takes over (the same rule as the home below: a literal, in EVERY
# state, independent of the query and of the clock).
check_door "Explorer" "Recherche Neezer"
# Bibliothèque draws a STABLE marker too (`LibraryScreen.kt`'s `LIBRARY_MARKER`,
# « Bibliothèque Neezer », a contentDescription on its root): the MB0 stub's text
# is gone now the lot paints the real screen, and the bottom bar's own item would
# open this door from any tab. The bar's WORD is « Biblio » since L1 (the SPA's
# own label, 72 dp on a 360 dp bar — « Bibliothèque » does not fit); the screen's
# marker keeps its full name.
check_door "Biblio" "Bibliothèque Neezer"
# M4 a supprimé le stub MB0 ; le marqueur est celui de l'écran, comme Accueil et
# Bibliothèque (`PodcastsEntry.kt`'s `PODCASTS_MARKER`).
check_door "Podcasts" "Podcasts Neezer"
# Accueil draws a STABLE marker (`HomeScreen.kt`'s `HOME_MARKER`, a
# contentDescription on the screen's root) instead of its MB0 stub text: the
# home's own copy names the weekday and the person, so a needle taken from it
# would fail in the evening or on another account. The marker is on the root in
# every state — hero, skeletons, a failed load with no server configured.
check_door "Accueil" "Accueil Neezer"

# Réglages, and the one line that proves the whole chain Kotlin -> JNA -> Rust:
# the version comes from `nz_version()`, not from a Kotlin constant.
#
# MB16 filled the screen: the MB0 placeholder (« Choisis une section. ») is gone,
# and the list is two levels now — the menu's filter is the literal that proves
# the first level, and *Assistance* (the last row, below the fold) is reached by
# ONE swipe, then shows the core's version. The needles are the screen's own, as
# MB6 did for the search.
if tap_text "Réglages" && await_text "Chercher un réglage" &&
    scroll_to_text "Assistance" && tap_text "Assistance" &&
    await_text "Noyau (Rust)"; then
    printf 'door=Réglages opened=1 rust_version=1\n'
    doors=$((doors + 1))
else
    printf 'door=Réglages opened=0\n'
    doors_failed=$((doors_failed + 1))
fi
check_alive "the Réglages door"

printf 'doors_opened=%s doors_failed=%s doors_expected=6\n' "$doors" "$doors_failed"

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
if [[ "${GITHUB_EVENT_NAME:-}" == pull_request ]]; then
    # 5 × ~14.4 s measured (the `await_text` budget plus the launch), and all
    # 5 print opened=0 by construction until MB2 writes `resolve_link`: on a PR
    # they cost 72 s and prove nothing. Master and tags still probe them.
    printf 'links_opened=0 links_probed=0 (app-link probes run on master and tags only: 5 × ~14 s, 0 opened until MB2)\n'
else
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
fi

if (( doors_failed > 0 )); then
    note "$doors_failed door(s) of the shell did not open"
    exit 1
fi

# THE RELEASE, SIGNED IN AND WALKED. The doors above prove a shell with no
# server; v1.6.5 (no platform TLS verifier after R8) and v1.6.6 (a crash on the
# home's first cover) both passed them. `release-journey.sh` drives the SAME
# release APK as a person does first: « Vérifier le certificat » on against a
# Let's Encrypt chain, a sign-in over HTTPS to `tools/perf_api.py --serve`, the
# home with real covers, then the eight doors of the app including
# « Réglages → Mode voiture » (the owner's crash on the minified build). It needs
# the server's Python packages (the workflow installs them); measured on v1.6.5
# and v1.6.6 it fails at the sign-in, and on the car fix's parent at the car
# door (the PR says with which line).
# `NEEZER_EVIDENCE_DIR` is the judge's evidence (lot T9): the workflow sets it,
# so the same walk also leaves the screenshots, the trees and the buffers the
# artifact `mobile-journey-evidence` carries. Unset on a developer machine, in
# which case the journey behaves exactly as T1 wrote it.
if [[ -n "${NEEZER_EVIDENCE_DIR:-}" ]]; then
    "$here/release-journey.sh" --serial "$serial" --evidence "$NEEZER_EVIDENCE_DIR"
else
    "$here/release-journey.sh" --serial "$serial"
fi
check_alive "the release journey"

# The instrumented step builds the DEBUG app, which links the Rust library: it
# needs the NDK, cargo-ndk and the three Rust targets — UNLESS the workflow's
# `apk` job handed the library and the bindings over (`native_artifacts_present`),
# in which case `-PskipRust` makes `:bridge` reuse them and this function is not
# called at all. A machine without them (a lot checking its own screen) still
# gets the whole toolchain installed here, idempotently, with a line saying what
# it installs.
prepare_android_build() {
    local sdk ndk_version="${NDK_VERSION:-30.0.16248370}"
    sdk="$(nz_sdk_root)"
    if [[ ! -d "$sdk/ndk/$ndk_version" ]]; then
        note "installing ndk;$ndk_version (the APK links libnz_ffi.so, built by cargo-ndk)"
        yes | "$sdk/cmdline-tools/latest/bin/sdkmanager" --licenses >/dev/null 2>&1 || true
        "$sdk/cmdline-tools/latest/bin/sdkmanager" "ndk;$ndk_version" >/dev/null
    fi
    export ANDROID_NDK_HOME="$sdk/ndk/$ndk_version"
    if ! command -v cargo-ndk >/dev/null 2>&1; then
        note "installing cargo-ndk 4.1.2"
        cargo install cargo-ndk --locked --version 4.1.2 >/dev/null 2>&1 ||
            note "cargo-ndk could not be installed: the instrumented step will fail at :bridge:buildRust"
    fi
    if command -v rustup >/dev/null 2>&1; then
        for target in aarch64-linux-android armv7-linux-androideabi x86_64-linux-android; do
            rustup target add "$target" >/dev/null 2>&1 || true
        done
    fi
}

# THE BENCH SERVERS THE INSTRUMENTED TESTS ASK FOR, so they measure instead of
# skipping. Two classes gate themselves behind `assumeTrue` when their fixture is
# absent, which is the honest thing to do on a developer machine but a silent hole
# in CI: run 45688339009 reported `(8 skipped)` in `:playback` and the job was
# green. Each fixture is a script the repo already ships and documents:
#
#  - `tone-server.py` on 4711 — the default `nzServer` of every
#    `NzMediaServiceTest` test: a real FLAC per title with Range, a 1×1 cover and
#    `/api/version`, so "the clock advanced" means "the engine rendered THAT"
#    (docs/agents/mobile.md, MB3c). 7 tests.
#  - `perf-server.sh` on 5610 — the default `SERVER` of `LanNodeLifecycleTest`:
#    it only needs `set_server` + `login(bench, Bench1)` to succeed, so the seed
#    is the smallest that logs in (200 titles), not the 8 000-track measure. 1 test.
#
# AutoBrowseTest (5595, an exact 200-favourite / 6-playlist / 120-title seed) and
# PartyDeviceTest (a party server + ffmpeg on the host) are NOT started here: their
# fixtures are the lots' own and stay skipped with their cause, never green by
# mistake. A server that fails to answer is a red job: a fixture CI owns must not
# degrade into a skip.
start_bench_servers() {
    local tone_server="$here/tone-server.py" attempt
    if [[ ! -f "$tone_server" ]]; then
        note "tone-server.py is missing: NzMediaServiceTest cannot measure"
        exit 1
    fi
    python3 "$tone_server" --port 4711 >"${NEEZER_EMU_STATE:-$HOME/.cache/neezer-emu}/tone-server.log" 2>&1 &
    bench_pids+=($!)
    for attempt in $(seq 1 100); do
        if curl -fsS "http://127.0.0.1:4711/api/version" >/dev/null 2>&1; then
            break
        fi
        if ! kill -0 "${bench_pids[-1]}" 2>/dev/null; then
            note "tone-server.py exited while starting; last lines:"
            tail -n 20 "${NEEZER_EMU_STATE:-$HOME/.cache/neezer-emu}/tone-server.log" >&2 || true
            exit 1
        fi
        sleep 0.2
    done
    curl -fsS "http://127.0.0.1:4711/api/version" >/dev/null 2>&1 || {
        note "tone-server.py never answered on 127.0.0.1:4711 (NzMediaServiceTest needs it)"
        exit 1
    }

    bench_perf_state="${NEEZER_EMU_STATE:-$HOME/.cache/neezer-emu}/perf-$$"
    mkdir -p "$bench_perf_state"
    # The smallest seed that logs in: `perf-server.sh` passes `--tracks 8000` before
    # its arguments and argparse takes the last value, so these override it.
    local out
    out="$(NEEZER_PERF_STATE="$bench_perf_state" "$here/perf-server.sh" start \
        --port 5610 --tracks 200 --favorites 20 --playlists 2 --big-playlist 20)" || {
        note "perf-server.sh did not start on 5610 (LanNodeLifecycleTest needs it)"
        exit 1
    }
    note "$out"
}

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
    gradle_args=()
    if native_artifacts_present; then
        printf 'native=prebuilt (libnz_ffi.so and the UniFFI Kotlin came with the workflow artifact: -PskipRust)\n'
        gradle_args+=("-PskipRust")
    else
        printf 'native=built here (no prebuilt libnz_ffi.so + bindings: the NDK, cargo-ndk and cargo run)\n'
        prepare_android_build
    fi

    # The APK the workflow handed over is the RELEASE build, whose versionCode is
    # its run number (the native line no longer carries an offset: the old app's
    # code belongs to another package); the DEBUG build
    # `connectedDebugAndroidTest` installs has versionCode 1, because VERSION_CODE
    # is not set in this job — so Android refuses it over the release one with
    # "INSTALL_FAILED_VERSION_DOWNGRADE". The smoke has already run against the
    # release APK, so the app is removed here and the instrumented step installs
    # its own.
    adb_dev uninstall fr.neezer.mobile >/dev/null 2>&1 || true

    # The fixture the `assumeTrue` gates would otherwise quietly skip (see
    # `start_bench_servers` above): on the host, reached from the guest at
    # `10.0.2.2`, stopped in `cleanup` whatever happens.
    step "bench servers (instrumented tests)"
    start_bench_servers

    tasks=()
    for module in "${test_modules[@]}"; do
        tasks+=("$module:connectedDebugAndroidTest")
    done
    # ONE INSTRUMENTATION AT A TIME. `org.gradle.parallel=true` (gradle.properties)
    # makes those tasks run at once, and they all drive the SAME emulator: the logs
    # of run 45253172024 show `:feature:viz`, `:feature:trackactions` and
    # `:playback` starting tests inside each other, and the suites then measure the
    # machine instead of themselves — `theBarsSceneFitsInOneFrame` read p95 28.52 ms
    # while it reads 2.14 ms alone (MB21), `PartyDeviceTest`'s card counted 0 request
    # instead of 1 on the rerun of 28c91f74, and the counts themselves came out as
    # "Tests 16/10 completed (8 skipped)". Gradle may still build the modules in
    # parallel; only the device work is serialised, because there is one device.
    # Only OUR device: a connected task otherwise installs and runs on every
    # emulator adb sees, including another lot's.
    ANDROID_SERIAL="$serial" nz_gradle_android --no-parallel "${tasks[@]}" "${gradle_args[@]}"
fi

step "done"
printf 'smoke_ok=1 doors_opened=%s boot_seconds=%s\n' "$doors" "$boot_seconds"
