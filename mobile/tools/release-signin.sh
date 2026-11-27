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
# `--handoff <dir>`: the caller (release-journey.sh) takes the bench over and keeps
# it until ITS last door, so the library and the next track's cover still reach a
# server after this script exits. Without it the bench is this script's alone and
# is stopped in `cleanup`, exactly as before.
handoff=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --serial) serial="$2"; shift 2 ;;
        --serial=*) serial="${1#*=}"; shift ;;
        --handoff) handoff="$2"; shift 2 ;;
        --handoff=*) handoff="${1#*=}"; shift ;;
        -h | --help) sed -n '2,40p' "$0"; exit 0 ;;
        *) printf 'release-signin.sh: unknown argument: %s\n' "$1" >&2; exit 2 ;;
    esac
done
[[ -n "$serial" ]] || { printf 'release-signin.sh: --serial is required\n' >&2; exit 2; }

# The accessibility tree, the app's pid and its crash cause live once, in ui.sh:
# `release-journey.sh` walks the same screens after this script and must tap them
# the same way.
# shellcheck source=mobile/tools/ui.sh
. "$here/ui.sh"

readonly tls_host=valid-isrgrootx1.letsencrypt.org
note() { printf 'release-signin.sh: %s\n' "$*" >&2; }
step() { printf '\n=== %s\n' "$*"; }

if [[ -n "$handoff" ]]; then
    work="$handoff"
    mkdir -p "$work"
else
    work="$(mktemp -d)"
fi
perf_state="$work/perf"
cleanup() {
    local code=$?
    # Avec `--handoff`, NI l'arrêt du banc NI la suppression de `$work` : le
    # certificat TLS et l'état du serveur sont ce que les portes suivantes
    # utilisent (un certificat supprimé fait échouer la poignée de main
    # suivante). C'est l'appelant qui arrête et nettoie.
    if [[ -z "$handoff" ]]; then
        NEEZER_PERF_STATE="$perf_state" "$here/perf-server.sh" stop >/dev/null 2>&1 || true
        rm -rf "$work"
    fi
    exit "$code"
}
trap cleanup EXIT

fresh_launch() {
    # A FRESH install's state (no server, no session, no pin): what a person who
    # just installed the release meets. The smoke before this one left a shell
    # that already walked its tabs.
    adb_dev shell pm clear "$ui_package" >/dev/null
    adb_dev logcat -c -b crash >/dev/null 2>&1 || true
    adb_dev shell am start -W -n "$(nz_launcher_component "$serial")" >/dev/null
    await_text "Adresse du serveur" 30 || {
        note "the setup screen never painted « Adresse du serveur »"
        app_crash_cause >&2
        exit 1
    }
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
    app_crash_cause >&2
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
        app_crash_cause >&2
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
if (( covers == 0 )); then
    note "signed in, but the home asked the server for no cover in 60 s"
    note "screen: $(screen_texts)"
    note "the server's last requests:"
    grep -E '"(GET|POST|PUT) /api/' "$access_log" | tail -n 15 >&2 || true
    adb_dev logcat -d 2>/dev/null | grep -E " (W|E) (neezer|NeezerImages|Coil)" | tail -n 15 >&2 || true
    exit 1
fi
# One more frame on the home after the covers arrived, and the same process.
dump_ui >/dev/null
[[ -n "$(app_pid)" ]] || { note "the app died while drawing its covers"; app_crash_cause >&2; exit 1; }
if [[ -n "$(adb_dev logcat -d -b crash 2>/dev/null | grep -F "Process: $ui_package" || true)" ]]; then
    note "the crash buffer names $ui_package:"
    app_crash_cause >&2
    exit 1
fi
printf 'release_signin=1 covers_served=%s seconds=%s\n' "$covers" "$((SECONDS - started))"
if [[ -n "$handoff" ]]; then
    printf 'release_handoff=%s port=%s\n' "$work" "$port"
fi
(( tls_ok == 1 )) || { note "signed in, but the platform trust store refused $tls_host (phase 1)"; exit 1; }
