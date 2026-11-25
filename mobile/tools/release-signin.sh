#!/usr/bin/env bash
# mobile/tools/release-signin.sh — the RELEASE APK, signed in over HTTPS, on its
# home screen with real covers (docs/agents/mobile.md, *Pitfalls*: "a release is
# proved signed in, not only opened").
#
#   mobile/tools/release-signin.sh --serial emulator-5580
#
# The app under test is whatever `fr.neezer.mobile` the device has: `ci-emulator.sh`
# calls this right after installing the universal RELEASE APK (minified by R8,
# exactly the file the release page serves), so what is proved here is the
# artifact a person installs — never a debug build.
#
# Why it exists. v1.6.5 and v1.6.6 both shipped with every CI job green:
#   - v1.6.5's minified build had lost the platform TLS verifier (R8), so a
#     server with a plain Let's Encrypt certificate was refused;
#   - v1.6.6 died on its home screen's first cover (`NoSuchMethodError` on
#     `CoilCovers.Image`, a `@Composable` compiled without the Compose compiler).
# Neither is visible on a screen with no server: the smoke opened the shell's tabs
# and stopped there. These two phases are what a person does first.
#
# Phase 1 — the platform verifier, through the setup screen, "Vérifier le
#   certificat" ON, against Let's Encrypt's own test host
#   `valid-isrgrootx1.letsencrypt.org` (leaf <- YR1 <- Root YR <- ISRG Root X1,
#   the chain a 2026 certificate carries). It is not a Neezer server, so the probe's
#   `GET /api/version` answers 404: « Le serveur a répondu 404 » is the proof the
#   handshake was ACCEPTED by Android's trust store. A refused certificate reads
#   « Le certificat du serveur … », a missing verifier reads as offline.
#
# Phase 2 — sign in and paint the home: `tools/perf_api.py --serve` on the host,
#   over HTTPS with a throwaway self-signed certificate (the emulator reaches the
#   host's loopback as 10.0.2.2), "Vérifier le certificat" OFF, the leaf accepted
#   on screen (TOFU), `bench` / `Bench1` (perf_api's published fixture), then the
#   home's marker AND at least one `GET /api/cover/…` answered 200 while the app
#   process is still alive. The covers are counted on the SERVER's access log: a
#   request exists only once the cover composable ran and Coil asked the core.
#
# Prints `release_tls_trusted=1`, `release_signin=1 covers_served=<n>` and the
# seconds each phase took; any failure names the screen text or the crash buffer.

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=mobile/tools/env.sh
. "$here/env.sh"

serial=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --serial) serial="$2"; shift 2 ;;
        --serial=*) serial="${1#*=}"; shift ;;
        -h | --help) sed -n '2,40p' "$0"; exit 0 ;;
        *) printf 'release-signin.sh: unknown argument: %s\n' "$1" >&2; exit 2 ;;
    esac
done
[[ -n "$serial" ]] || { printf 'release-signin.sh: --serial is required\n' >&2; exit 2; }

# Not `package`: env.sh's functions declare a `local package`, which a readonly
# global of that name turns into an error.
readonly app_package=fr.neezer.mobile
readonly tls_host=valid-isrgrootx1.letsencrypt.org
adb_dev() { "$(nz_adb)" -s "$serial" "$@"; }
note() { printf 'release-signin.sh: %s\n' "$*" >&2; }
step() { printf '\n=== %s\n' "$*"; }

work="$(mktemp -d)"
perf_state="$work/perf"
cleanup() {
    local code=$?
    NEEZER_PERF_STATE="$perf_state" "$here/perf-server.sh" stop >/dev/null 2>&1 || true
    rm -rf "$work"
    exit "$code"
}
trap cleanup EXIT

# --- the accessibility tree (the same nodes TalkBack reads) ------------------

dump_ui() {
    local attempt
    for attempt in 1 2 3 4 5; do
        if adb_dev shell uiautomator dump /sdcard/nz-signin.xml >/dev/null 2>&1; then
            adb_dev shell cat /sdcard/nz-signin.xml 2>/dev/null
            return 0
        fi
    done
    return 1
}

# Waits until the tree contains <needle> (a text or a content description).
await_text() {
    local needle="$1" budget="${2:-20}" started=$SECONDS
    while (( SECONDS - started < budget )); do
        if dump_ui | grep -qF -- "$needle"; then
            return 0
        fi
    done
    return 1
}

# The keyboard a field opened covers the lower half of the form, and a node under
# it is not in the tree at all (measured on the first run of this script: « Vérifier
# le serveur » was "absent" behind the IME). BACK hides it — and only when it is
# shown, since a BACK with no keyboard leaves the screen.
hide_keyboard() {
    if adb_dev shell dumpsys input_method 2>/dev/null | grep -qE "mInputShown=true|isInputViewShown=true"; then
        adb_dev shell input keyevent KEYCODE_BACK >/dev/null
    fi
}

# The centre of the LAST node whose text is exactly <label> (a button's text sits
# inside the clickable node, so its centre is on the control). A control below the
# fold is brought up by at most five swipes — gestures the device performed, never
# a sleep.
tap_text() {
    local label="$1" bounds x1 y1 x2 y2 attempt width height
    hide_keyboard
    for attempt in 1 2 3 4 5 6; do
        bounds="$(dump_ui | tr '>' '\n' | grep -F -- "text=\"$label\"" |
            sed -n 's/.*bounds="\[\([0-9]*\),\([0-9]*\)\]\[\([0-9]*\),\([0-9]*\)\]".*/\1 \2 \3 \4/p' | tail -1)"
        [[ -n "$bounds" ]] && break
        read -r width height < <(adb_dev shell wm size | sed -n 's/.*: \([0-9]*\)x\([0-9]*\).*/\1 \2/p' | tail -1)
        adb_dev shell input swipe $((width / 2)) $((height * 7 / 10)) $((width / 2)) $((height * 3 / 10)) 240 >/dev/null
    done
    [[ -n "$bounds" ]] || return 1
    read -r x1 y1 x2 y2 <<<"$bounds"
    adb_dev shell input tap $(((x1 + x2) / 2)) $(((y1 + y2) / 2)) >/dev/null
}

# Types <text> into the <n>-th text field of the screen (1-based, tree order):
# a Compose field is an EditText node. The field's own text is the proof — a
# field that has not taken the focus yet drops injected keys while `input text`
# reports success (MQ2's journey measured exactly that).
type_into() {
    local index="$1" text="$2" attempt bounds x1 y1 x2 y2
    for attempt in 1 2 3; do
        hide_keyboard
        bounds="$(dump_ui | tr '>' '\n' | grep -F 'class="android.widget.EditText"' |
            sed -n 's/.*bounds="\[\([0-9]*\),\([0-9]*\)\]\[\([0-9]*\),\([0-9]*\)\]".*/\1 \2 \3 \4/p' |
            sed -n "${index}p")"
        [[ -n "$bounds" ]] || { note "no text field #$index on screen"; return 1; }
        read -r x1 y1 x2 y2 <<<"$bounds"
        adb_dev shell input tap $(((x1 + x2) / 2)) $(((y1 + y2) / 2)) >/dev/null
        adb_dev shell input text "$text" >/dev/null
        # The password field shows dots, never the text: its proof is the login.
        if [[ "${3:-}" == secret ]] || dump_ui | grep -qF -- "text=\"$text\""; then
            return 0
        fi
    done
    note "the field #$index never held « $text »"
    return 1
}

app_pid() { adb_dev shell pidof "$app_package" 2>/dev/null | tr -d '\r'; }

crash_cause() {
    adb_dev logcat -d -b crash 2>/dev/null | grep -E "FATAL EXCEPTION|Error:|Exception:" | head -4 || true
}

fresh_launch() {
    # A FRESH install's state (no server, no session, no pin): what a person who
    # just installed the release meets. The smoke before this one left a shell
    # that already walked its tabs.
    adb_dev shell pm clear "$app_package" >/dev/null
    adb_dev logcat -c -b crash >/dev/null 2>&1 || true
    adb_dev shell am start -W -n "$(nz_launcher_component "$serial")" >/dev/null
    await_text "Adresse du serveur" 30 || {
        note "the setup screen never painted « Adresse du serveur »"
        crash_cause >&2
        exit 1
    }
}

# Every visible text of the screen, for a failure message that says what was there.
screen_texts() {
    dump_ui | tr '>' '\n' | sed -n 's/.* text="\([^"]\+\)".*/\1/p' | head -30 | paste -sd '|' -
}

# --- phase 1: the platform verifier answers on the minified build -------------

step "release: the platform trust store accepts a Let's Encrypt chain (« Vérifier le certificat » on)"
started=$SECONDS
fresh_launch
type_into 1 "$tls_host"
tap_text "Vérifier le serveur" || { note "no « Vérifier le serveur » button"; exit 1; }
# A refusal does not stop the run: phase 2 still says whether the sign-in and the
# covers work, and the job fails at the end with both verdicts printed.
tls_ok=0
if await_text "Le serveur a répondu 404" 40; then
    tls_ok=1
    printf 'release_tls_trusted=1 host=%s seconds=%s\n' "$tls_host" "$((SECONDS - started))"
else
    printf 'release_tls_trusted=0 host=%s\n' "$tls_host"
    note "Android's trust store did not accept $tls_host on the release build"
    note "screen: $(screen_texts)"
    # The core's own words for the refusal (tag `neezer`), and the verifier's.
    adb_dev logcat -d 2>/dev/null | grep -iE "rustls|platformverifier|NeezerPlatform|certificate|nz_api" | tail -n 25 >&2 || true
    crash_cause >&2
fi

# --- phase 2: sign in over HTTPS and paint the home with covers ---------------

step "release: sign in over HTTPS, then the home paints real covers"
started=$SECONDS
openssl req -x509 -newkey rsa:2048 -nodes -days 2 -subj "/CN=neezer-release-smoke" \
    -addext "subjectAltName=IP:10.0.2.2,IP:127.0.0.1" \
    -keyout "$work/key.pem" -out "$work/cert.pem" >/dev/null 2>&1
# A SMALL library (the measure is "does it sign in and draw", not a bench): 200
# titles, a dozen releases whose sleeves are real JPEGs (`--covers`).
server_out="$(NEEZER_PERF_STATE="$perf_state" "$here/perf-server.sh" start \
    --tracks 200 --playlists 4 --favorites 40 --big-playlist 40 --releases 12 \
    --covers "$(nz_repo_root)/tests/assets/cover.jpg" \
    --tls-cert "$work/cert.pem" --tls-key "$work/key.pem")"
port="$(sed -n 's/^port=\([0-9]*\) .*/\1/p' <<<"$server_out")"
[[ -n "$port" ]] || { note "the bench server printed no port: $server_out"; exit 1; }
access_log="$perf_state/last.log"

fresh_launch
type_into 1 "10.0.2.2:$port"
# OFF: the bench's leaf is self-signed, so it is accepted on screen (trust on
# first use) — the switch's own label is the LAST node with this text.
tap_text "Vérifier le certificat" || { note "no « Vérifier le certificat » switch"; exit 1; }
tap_text "Vérifier le serveur" || { note "no « Vérifier le serveur » button"; exit 1; }
await_text "Certificat auto-signé" 30 || {
    note "no certificate card for the bench server; screen: $(screen_texts)"
    exit 1
}
# The card's button sits below the fold on a phone: tap_text scrolls to it.
tap_text "Accepter ce certificat" || { note "no « Accepter ce certificat » button; screen: $(screen_texts)"; exit 1; }
await_text "Ton compte" 30 || { note "the account form never opened; screen: $(screen_texts)"; exit 1; }
type_into 1 bench
type_into 2 Bench1 secret
tap_text "Se connecter" || { note "no « Se connecter » button"; exit 1; }

# The home, then its covers: both while the process is alive. A crash on the
# first cover (v1.6.6) kills the process before any `/api/cover` request exists.
covers=0
budget_started=$SECONDS
signed_in=0
while (( SECONDS - budget_started < 60 )); do
    if [[ -z "$(app_pid)" ]]; then
        note "the app died after signing in"
        crash_cause >&2
        exit 1
    fi
    if (( signed_in == 0 )) && dump_ui | grep -qF "Accueil Neezer"; then
        signed_in=1
    fi
    covers="$(grep -cE '"GET /api/cover/[^ ]+ HTTP/[0-9.]+" 200' "$access_log" || true)"
    if (( signed_in == 1 && covers > 0 )); then
        break
    fi
done
(( signed_in == 1 )) || { note "the home never painted after signing in; screen: $(screen_texts)"; exit 1; }
(( covers > 0 )) || { note "signed in, but the home asked the server for no cover in 60 s"; exit 1; }
# One more frame on the home after the covers arrived, and the same process.
dump_ui >/dev/null
[[ -n "$(app_pid)" ]] || { note "the app died while drawing its covers"; crash_cause >&2; exit 1; }
if [[ -n "$(adb_dev logcat -d -b crash 2>/dev/null | grep -F "Process: $app_package" || true)" ]]; then
    note "the crash buffer names $app_package:"
    crash_cause >&2
    exit 1
fi
printf 'release_signin=1 covers_served=%s seconds=%s\n' "$covers" "$((SECONDS - started))"
(( tls_ok == 1 )) || { note "signed in, but the platform trust store refused $tls_host (phase 1)"; exit 1; }
