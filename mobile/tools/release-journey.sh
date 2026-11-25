#!/usr/bin/env bash
# mobile/tools/release-journey.sh — the RELEASE APK's first-run JOURNEY, on the
# artifact a person installs: sign in, then every main screen, then the car
# settings.
#
#   mobile/tools/release-journey.sh --serial emulator-5580
#
# It signs in first (`release-signin.sh`: the platform trust store, then HTTPS,
# then the home with real covers) and then walks the screens a person uses:
#
#   Accueil → Rechercher → Bibliothèque → Podcasts → Accueil
#   → Réglages → Lecture & son → Mode voiture
#
# Eight doors. Each one is a tap plus the literal ONLY that screen draws, so a
# tab that opens the previous screen, a screen that silently fails to load, or a
# route that never arrives fails here instead of passing as "the shell opened".
# After every step the process must still be `pidof fr.neezer.mobile`, and
# neither buffer may name a crash or an ANR of ours (`FATAL EXCEPTION`,
# `UnsatisfiedLinkError`, `ANR in fr.neezer.mobile`) — a crash is a failure of
# the journey, never a result.
#
# Why it exists. `release-signin.sh` closed the two defects the shell smoke could
# not see (v1.6.5: the platform verifier stripped by R8; v1.6.6: a crash on the
# home's first cover). The third one, the owner's, was a door: « Réglages → Mode
# voiture » died before its first frame on the minified build, and every CI job
# stayed green because nothing OPENED it. This script opens it, on that build.
#
# Prints `release_journey=1 doors=8/8 car_settings=1 crash=0 seconds=<n>`; on a
# failure it prints the door that failed, the screen's own texts, the last tree
# (bounded) and the crash buffer's cause, then exits 1.
#
# Bounded everywhere: `await_text` 10 s (30 s for a cold launch), eight swipes at
# most for a below-the-fold control, no `sleep`. The emulator is not started
# here — `ci-emulator.sh` owns it, and every `adb` call names the serial it was
# given, never a device this script did not choose.

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
        *) printf 'release-journey.sh: unknown argument: %s\n' "$1" >&2; exit 2 ;;
    esac
done
[[ -n "$serial" ]] || { printf 'release-journey.sh: --serial is required\n' >&2; exit 2; }

# shellcheck source=mobile/tools/ui.sh
. "$here/ui.sh"

note() { printf 'release-journey.sh: %s\n' "$*" >&2; }
step() { printf '\n=== %s\n' "$*"; }

readonly doors_expected=8
doors=0
car_settings=0
started=$SECONDS

# The numbers a CI log is read for, printed once — on the way out of a failure
# too, so `car_settings=0` is visible next to the cause instead of replaced by it.
verdict() {
    local crash=0
    [[ -n "$(app_crash_cause)" ]] && crash=1
    printf 'release_journey=%s doors=%s/%s car_settings=%s crash=%s seconds=%s\n' \
        "$1" "$doors" "$doors_expected" "$car_settings" "$crash" "$((SECONDS - started))"
}

# Every step's second half: the app is still the app, and nothing crashed.
check_app() {
    local where="$1" cause
    check_alive "$where"
    if [[ -z "$(app_pid)" ]]; then
        verdict 0
        fail_screen "the app's process is gone after $where"
    fi
    cause="$(app_crash_cause)"
    if [[ -n "$cause" ]]; then
        verdict 0
        fail_screen "the crash buffer names a crash after $where"
    fi
}

# One door: tap <label>, wait for <needle>, then prove the process survives it.
open_door() {
    local name="$1" label="$2" needle="$3" budget="${4:-10}"
    if ! tap_text "$label"; then
        verdict 0
        fail_screen "no node « $label » to tap for the $name door"
    fi
    if await_text "$needle" "$budget"; then
        printf 'door=%s opened=1\n' "$name"
        doors=$((doors + 1))
    else
        printf 'door=%s opened=0 expected="%s"\n' "$name" "$needle"
        verdict 0
        fail_screen "the $name door did not paint « $needle »"
    fi
    check_app "the $name door"
}

# A screen prepared as a person finds it: awake, keyguard down, no ANR dialog.
prepare_device

# --- 1. signed in, on the home, with covers ---------------------------------
# `release-signin.sh` prints its own verdicts; they are kept in a file as well so
# the cover count below is read from the run and not from a grep of the console.
step "release journey: sign in first (release-signin.sh)"
signin_log="$(mktemp)"
trap 'rm -f "$signin_log"' EXIT
if ! "$here/release-signin.sh" --serial "$serial" 2>&1 | tee "$signin_log"; then
    verdict 0
    note "the release sign-in failed; the journey starts from a signed-in home"
    exit 1
fi
grep -q '^release_tls_trusted=1' "$signin_log" || {
    verdict 0
    fail_screen "the platform trust store did not accept the Let's Encrypt chain (phase 1)"
}
covers="$(sed -n 's/^release_signin=1 covers_served=\([0-9]*\).*/\1/p' "$signin_log" | tail -1)"
[[ -n "$covers" ]] && (( covers >= 1 )) || {
    verdict 0
    fail_screen "signed in, but the home asked the server for no cover"
}

# The walk's crash/ANR verdicts are about the walk: the buffers are emptied here
# so a line from the sign-in (already judged by release-signin.sh) cannot fail a
# later door for a reason that is not its own.
adb_dev logcat -c >/dev/null 2>&1 || true

step "release journey: the screens"
# The home is already painted by the sign-in; its marker is on the root in every
# state (hero, skeletons, a failed load), so this door costs no tap.
if await_text "Accueil Neezer" 30; then
    printf 'door=Accueil opened=1 covers_served=%s\n' "$covers"
    doors=$((doors + 1))
else
    printf 'door=Accueil opened=0 expected="Accueil Neezer"\n'
    verdict 0
    fail_screen "the home never painted « Accueil Neezer » after signing in"
fi
check_app "the Accueil door"

open_door "Rechercher" "Rechercher" "Recherche Neezer"
open_door "Bibliothèque" "Bibliothèque" "Bibliothèque Neezer"
open_door "Podcasts" "Podcasts" "Podcasts Neezer"
open_door "Accueil (retour)" "Accueil" "Accueil Neezer"

# --- 2. Réglages → Lecture & son → Mode voiture ------------------------------
# Réglages is the phone shell's top-bar action (the bottom bar has the four tabs
# only); its menu is proved by its filter's own label.
open_door "Réglages" "Réglages" "Chercher un réglage"

# « Qualité demandée au serveur » is a card title ONLY `PlaybackSection` draws:
# waiting for « Lecture & son » alone would match the menu row that was just
# tapped, and a tap that did nothing would pass.
open_door "Lecture & son" "Lecture & son" "Qualité demandée au serveur"

# The car-settings card is the LAST of that section, behind the quality card and
# the five effect cards (output, pan, audio, equaliser with its ten faders,
# transitions): eight bounded swipes, along the margin so the faders' own drags
# are never touched. Its button is the one gate M13 left; the Activity draws
# « Déclenchement », which no other screen has.
expected_needle="Déclenchement"
if ! scroll_to_text "Mode voiture" 8; then
    printf 'door=Mode voiture opened=0 expected="%s"\n' "$expected_needle"
    verdict 0
    fail_screen "no « Mode voiture » card in Lecture & son"
fi
# The button asks for POST_NOTIFICATIONS (API 33+) and opens the Activity in the
# same click: the permission dialog would cover the Activity and the dump would
# describe the dialog. `ci-emulator.sh` installs with `-g`, but `pm clear` in the
# sign-in can drop a runtime grant; granting it here restores the installer's
# state and changes nothing the journey is meant to prove.
adb_dev shell pm grant "$ui_package" android.permission.POST_NOTIFICATIONS >/dev/null 2>&1 || true
if ! tap_text "Ouvrir les réglages du mode voiture"; then
    printf 'door=Mode voiture opened=0 expected="%s"\n' "$expected_needle"
    verdict 0
    fail_screen "no « Ouvrir les réglages du mode voiture » button"
fi
if await_text "$expected_needle" 15; then
    printf 'door=Mode voiture opened=1\n'
    doors=$((doors + 1))
    car_settings=1
else
    printf 'door=Mode voiture opened=0 expected="%s"\n' "$expected_needle"
    verdict 0
    fail_screen "the car settings did not paint « $expected_needle »"
fi
check_app "the Mode voiture door"

# BACK finishes the Activity (it is started NEW_TASK from the settings screen).
# The pane returns at whatever offset it was left at, so either of its own card
# titles names it — the car card just tapped, or the quality card at the top if
# the scroll state was rebuilt; neither exists on the car Activity.
adb_dev shell input keyevent KEYCODE_BACK >/dev/null
if ! await_text "Ouvrir les réglages du mode voiture" 6 &&
    ! await_text "Qualité demandée au serveur" 10; then
    verdict 0
    fail_screen "BACK from the car settings did not return to Lecture & son"
fi
if ! tap_text "Accueil" || ! await_text "Accueil Neezer" 15; then
    verdict 0
    fail_screen "the home did not come back after the car settings"
fi
check_app "the return to Accueil"

verdict 1
