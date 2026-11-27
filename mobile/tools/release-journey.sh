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
# `--evidence <dir>` (lot T9) adds the judge's evidence and NOTHING else: one
# screenshot + one accessibility dump per door (01…07), the four player captures
# (08…11), then `journey.txt`, `logcat.txt`, `crash.txt`, `device.txt` whatever
# happened. Without the flag not one capture is taken — T1's numbers, byte for
# byte.
#
# Bounded everywhere: `await_text` 10 s (30 s for a cold launch), eight swipes at
# most for a below-the-fold control, no `sleep` — the ONE exception is inside the
# mid-gesture capture of `--evidence`, where the device itself is performing a
# 3 s swipe and the shot has to land inside it. The emulator is not started
# here — `ci-emulator.sh` owns it, and every `adb` call names the serial it was
# given, never a device this script did not choose.

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=mobile/tools/env.sh
. "$here/env.sh"

serial=""
evidence=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --serial) serial="$2"; shift 2 ;;
        --serial=*) serial="${1#*=}"; shift ;;
        --evidence) evidence="$2"; shift 2 ;;
        --evidence=*) evidence="${1#*=}"; shift ;;
        -h | --help) sed -n '2,40p' "$0"; exit 0 ;;
        *) printf 'release-journey.sh: unknown argument: %s\n' "$1" >&2; exit 2 ;;
    esac
done
[[ -n "$serial" ]] || { printf 'release-journey.sh: --serial is required\n' >&2; exit 2; }

# shellcheck source=mobile/tools/ui.sh
. "$here/ui.sh"

note() { printf 'release-journey.sh: %s\n' "$*" >&2; }
step() { printf '\n=== %s\n' "$*"; }

# --- the judge's evidence (lot T9) ------------------------------------------
# A capture is ONE screenshot and the accessibility dump of the SAME screen:
# the judge looks at the pixels, `precheck.py` reads the tree, and a review can
# cite both. `exec-out` and not `shell`: `screencap -p` through a pty rewrites
# its CR/LF pairs and corrupts the PNG (the lot's escalation says so).
shot() {
    [[ -n "$evidence" ]] || return 0
    local name="$1"
    mkdir -p "$evidence/screens"
    adb_dev exec-out screencap -p >"$evidence/screens/$name.png" ||
        note "screencap failed for $name"
    dump_ui >"$evidence/screens/$name.xml" ||
        note "dump_ui failed for $name"
    printf 'shot=%s\n' "$name"
}

# The four files the judge reads around the images: the script's own output,
# the buffers, and the device the captures came from. Written on EVERY exit —
# the crash is the thing worth judging — never only on success.
evidence_finish() {
    [[ -n "$evidence" ]] || return 0
    mkdir -p "$evidence"
    adb_dev logcat -d -b all 2>/dev/null | tail -n 5000 >"$evidence/logcat.txt" || true
    adb_dev logcat -d -b all 2>/dev/null |
        grep -E 'FATAL EXCEPTION|ANR in fr.neezer.mobile|UnsatisfiedLinkError|dlopen failed' \
            >"$evidence/crash.txt" || true
    {
        printf 'model=%s\n' "$(adb_dev shell getprop ro.product.model 2>/dev/null | tr -d '\r')"
        printf 'sdk=%s\n' "$(adb_dev shell getprop ro.build.version.sdk 2>/dev/null | tr -d '\r')"
        printf 'density=%s\n' "$(adb_dev shell getprop ro.sf.lcd_density 2>/dev/null | tr -d '\r')"
        printf 'font_scale=%s\n' "$(adb_dev shell settings get system font_scale 2>/dev/null | tr -d '\r')"
        printf 'serial=%s\n' "$serial"
    } >"$evidence/device.txt" 2>/dev/null || true
}

# `journey.txt` is what this script prints, taken from the stream itself: a
# redirect at the call site would lose the `set -e` failure paths.
if [[ -n "$evidence" ]]; then
    mkdir -p "$evidence/screens"
    exec > >(tee "$evidence/journey.txt")
fi

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

# --- the player, for the judge's evidence (lot T9) ---------------------------
# Four captures no door needs, taken on the home the SIGN-IN has just painted
# (see the call site): they are EVIDENCE-ONLY, the journey fails on them for a
# crash (T1's rule), never for a missing mini-bar — a screen the build cannot
# reach is `player=unavailable`, the judge then says INCOMPLETE, and nobody
# pretends a flake is a defect.

# A control named by `content-desc` rather than by its text: Compose draws an
# icon button with an empty `text` and its label in the description TalkBack
# reads (the home's cards: `Lire ${title}, ${artist}`; the mini bar:
# `Ouvrir le lecteur plein écran`). The tap lands on the node's CENTRE: these
# are full cards and bar buttons, not a row found at the bottom of a pane.
tap_desc_prefix() {
    local prefix="$1" budget="${2:-15}" started=$SECONDS line bounds x1 y1 x2 y2
    while (( SECONDS - started < budget )); do
        # `|| true`: no match is the normal first state, and a failed
        # substitution must not end the walk under `set -e` (the helper is
        # called from an `if !`, which is what made the first version survive).
        line="$(ui_text | tr '>' '\n' | grep -F "content-desc=\"$prefix" | head -1 || true)"
        if [[ -n "$line" ]]; then
            bounds="$(sed -n 's/.*bounds="\[\([0-9]*\),\([0-9]*\)\]\[\([0-9]*\),\([0-9]*\)\]".*/\1 \2 \3 \4/p' <<<"$line")"
            if [[ -n "$bounds" ]]; then
                read -r x1 y1 x2 y2 <<<"$bounds"
                adb_dev shell input tap $(((x1 + x2) / 2)) $(((y1 + y2) / 2)) >/dev/null
                return 0
            fi
        fi
        # No `sleep`: `ui_text` is a `uiautomator dump`, which waits for idle
        # and takes the better part of a second — the loop is paced by the
        # device, and the budget above is the bound.
    done
    return 1
}

# Every word on the screen, minus the clock and the level a track redraws every
# second: two sets that differ name a real change (the title), not the elapsed
# time between the two dumps.
player_words() {
    ui_text | tr '>' '\n' | sed -n 's/.* text="\([^"]\+\)".*/\1/p' |
        grep -vE '^[0-9]+:[0-9]{2}$|^[0-9]+ %$|^-?[0-9]+$|^[0-9]+$' | sort -u
}

player_evidence() {
    local before after waited bounds x1 y1 x2 y2 width height x y0 y_end swipe

    # From the home, the FIRST card that offers to play: a rule read from the
    # tree, never a coordinate.
    if ! tap_desc_prefix "Lire " 20; then
        printf 'player=unavailable cause=no "Lire " card on the home\n'
        printf 'player=unavailable screen=%s\n' "$(screen_texts)"
        return 0
    fi
    if ! await_text "Ouvrir le lecteur plein écran" 15; then
        printf 'player=unavailable cause=the mini bar never named itself\n'
        return 0
    fi
    if ! tap_desc_prefix "Ouvrir le lecteur plein écran"; then
        printf 'player=unavailable cause=no mini bar to open\n'
        return 0
    fi
    if ! await_text "Réduire le lecteur" 15; then
        printf 'player=unavailable cause=the full-screen player never painted\n'
        return 0
    fi
    shot 08-lecteur

    # The next track: the title (and the sleeve) change, the layout does not.
    before="$(player_words)"
    if ! tap_desc_prefix "Suivant" && ! tap_text "Suivant"; then
        printf 'player=unavailable cause=no "Suivant" control on the player\n'
        return 0
    fi
    waited=0
    while (( waited < 10 )); do
        after="$(player_words)"
        if [[ "$before" != "$after" ]]; then
            break
        fi
        sleep 1
        waited=$((waited + 1))
    done
    if [[ "$before" == "$after" ]]; then
        printf 'player=unavailable cause=the title did not change after Suivant\n'
        return 0
    fi
    shot 09-lecteur-suivant

    # The queue sheet, then back to the player.
    if ! tap_desc_prefix "File d'attente" && ! tap_text "File d'attente"; then
        printf 'player=unavailable cause=no queue button on the player\n'
        return 0
    fi
    if await_text "Vider la file" 10; then
        shot 10-file
    else
        printf 'player=unavailable cause=the queue sheet never painted\n'
    fi
    # The queue is a SHEET, not an Activity: its own close control returns to the
    # player. BACK closed the PLAYER instead (measured on run 45803188631), which
    # is why the close button, not the key, is the path.
    if ! tap_desc_prefix "Fermer la file d'attente" 5; then
        adb_dev shell input keyevent KEYCODE_BACK >/dev/null
    fi
    if ! await_text "Réduire le lecteur" 10; then
        printf 'player=unavailable cause=the queue did not close back to the player\n'
        return 0
    fi

    # The mid-gesture capture: the header's bounds give the finger's ROW; the
    # finger starts at the middle of the screen (a press on a button is not a
    # drag) and goes DOWN — the only direction the sheet accepts at rest
    # (NowPlaying.kt: "an upward one has nowhere to go"). The swipe lasts 3 s, so
    # a shot at 1.6 s lands mid-drag BY CONSTRUCTION: 53 % of the 1 440 px path
    # is 43 % of the 676 dp sheet, above its 28 % dismissal threshold, and the
    # release finishes the run. The `sleep` is inside a gesture the device is
    # performing, and the row is the control's own — the two rules the plan
    # forbids elsewhere do not apply here.
    bounds="$(grep -F 'Réduire le lecteur' "$evidence/screens/08-lecteur.xml" | head -1 |
        sed -n 's/.*bounds="\[\([0-9]*\),\([0-9]*\)\]\[\([0-9]*\),\([0-9]*\)\]".*/\1 \2 \3 \4/p')"
    if [[ -z "$bounds" ]]; then
        printf 'player=unavailable cause=no header bounds in 08-lecteur.xml\n'
        return 0
    fi
    # The queue's veil keeps the POINTER for the length of its fade (NzSheet: a
    # full-screen clickable Box inside an AnimatedVisibility, a SIBLING of the
    # sheet): a DOWN landing on it never reaches the player's gesture, so the
    # sheet does not move at all. `await_text "Réduire le lecteur"` is true the
    # whole time — the player is UNDER the queue — so the previous head started
    # the drag on the veil: measured on run 45928645086, where 11 came out
    # byte-identical to 09 in both directions. Wait for the queue's own title to
    # leave the tree (the end of the exit animation), then drag.
    queue_gone=0
    for _ in $(seq 1 100); do
        if ! ui_text | grep -qF "File d'attente"; then queue_gone=1; break; fi
        sleep 0.1
    done
    (( queue_gone == 1 )) || printf 'player=the queue sheet is still in the tree\n'
    read -r x1 y1 x2 y2 <<<"$bounds"
    read -r width height < <(ui_screen)
    x=$((width / 2))
    y0=$(((y1 + y2) / 2))
    y_end=$((y0 + height * 6 / 10))
    adb_dev shell input swipe "$x" "$y0" "$x" "$y_end" 3000 >/dev/null &
    swipe=$!
    sleep 1.6
    adb_dev exec-out screencap -p >"$evidence/screens/11-lecteur-glisse.png"
    printf 'shot=11-lecteur-glisse direction=bas\n'
    wait "$swipe" 2>/dev/null || true
    # The tree is taken AFTER the gesture on purpose: uiautomator waits for
    # inactivity, so during a drag it returns the last stable state — which is
    # how 11 came out identical to 09. After the release the sheet is closed and
    # the tree is the home's; it differs from 09, exactly what C4 asks ("the
    # gesture had an effect").
    sleep 1
    dump_ui >"$evidence/screens/11-lecteur-glisse.xml" || true
    if cmp -s "$evidence/screens/09-lecteur-suivant.xml" \
              "$evidence/screens/11-lecteur-glisse.xml"; then
        verdict 0
        fail_screen "le glissement du lecteur n'a rien fait (11 = 09)"
    fi
    if [[ -z "$(app_pid)" ]]; then
        verdict 0
        fail_screen "the app's process is gone after the downward drag"
    fi
}

# --- 1. signed in, on the home, with covers ---------------------------------
# `release-signin.sh` prints its own verdicts; they are kept in a file as well so
# the cover count below is read from the run and not from a grep of the console.
step "release journey: sign in first (release-signin.sh)"
# Le banc vit aussi longtemps que le parcours : sans lui, la bibliothèque peint
# « Hors ligne » et la pochette du titre suivant n'arrive jamais (mesuré : 03 et
# 08/09 des runs 45818870854 et 45878251375). `release-signin.sh` arrête le sien
# à la sortie ; ici on lui donne `$walk` en main et c'est ce trap qui le rend.
walk="$(mktemp -d)"
handoff_cleanup() {
    NEEZER_PERF_STATE="$walk/perf" "$here/perf-server.sh" stop >/dev/null 2>&1 || true
    rm -rf "$walk"
}
signin_log="$(mktemp)"
# UN seul trap : bash remplace le précédent, et `rm -f "$signin_log"` avec lui.
trap 'handoff_cleanup; rm -f "$signin_log"; evidence_finish' EXIT
if ! "$here/release-signin.sh" --serial "$serial" --handoff "$walk" 2>&1 | tee "$signin_log"; then
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
shot 01-accueil

# The player's four captures happen HERE, on the home the sign-in has just
# painted: the bench now survives the sign-in (`--handoff`, above), but the home
# is still the one state with its « Lire … » cards already loaded and no tap in
# between, so the player is opened from it; the sheet is dismissed before the
# walk continues.
if [[ -n "$evidence" ]]; then
    player_evidence
    # Back on the home whatever the player did: the drag closes the sheet, a
    # failure leaves it open, and the next door is behind it.
    if ui_text | grep -qF "Réduire le lecteur"; then
        tap_desc_prefix "Réduire le lecteur" 5 || true
    fi
    # Evidence-only steps never fail the journey (T1's rule): if the sheet will
    # not close, say so and let the next door fail on its own terms.
    if ! await_text "Accueil Neezer" 10; then
        printf 'player=recovery the home did not come back after the captures\n'
        tap_desc_prefix "Réduire le lecteur" 5 || true
        await_text "Accueil Neezer" 10 ||
            printf 'player=recovery still not on the home\n'
    fi
fi

open_door "Rechercher" "Rechercher" "Recherche Neezer"
shot 02-rechercher
open_door "Bibliothèque" "Biblio" "Bibliothèque Neezer"
shot 03-bibliotheque
# Un échec nomme sa cause (règle 2) : « Hors ligne » alors que le banc tourne est
# un défaut du parcours, pas une réponse de la personne.
if ui_text | grep -qF "Hors ligne"; then
    verdict 0
    fail_screen "la bibliothèque est hors ligne alors que le banc tourne"
fi
open_door "Podcasts" "Podcasts" "Podcasts Neezer"
shot 04-podcasts
open_door "Accueil (retour)" "Accueil" "Accueil Neezer"

# --- 2. Réglages → Lecture & son → Mode voiture ------------------------------
# Réglages is the bottom bar's FIFTH item (L1 removed the top bar; it used to be
# the top-bar action); its menu is proved by its filter's own label. The
# Bibliothèque tab's WORD is « Biblio » (the SPA's label: 72 dp on a 360 dp bar),
# while the screen keeps its full name.
open_door "Réglages" "Réglages" "Chercher un réglage"
shot 05-reglages

# « Qualité demandée au serveur » is a card title ONLY `PlaybackSection` draws:
# waiting for « Lecture & son » alone would match the menu row that was just
# tapped, and a tap that did nothing would pass.
open_door "Lecture & son" "Lecture & son" "Qualité demandée au serveur"
shot 06-lecture-et-son

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
shot 07-mode-voiture

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
